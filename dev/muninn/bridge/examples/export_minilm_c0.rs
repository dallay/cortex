//! Private C0 candidate export. No acceptance policy or numerical verdict.

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod supported {
    use muninn_bridge_spike::{
        artifact_fs::{staging::model::VerifiedArtifactCache, ArtifactDir},
        minilm_encoder::CandidateCpuSentenceEncoder,
        minilm_tokenizer::MiniLmTokenizer,
        model_manifest::PinnedMiniLmManifest,
    };
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::{collections::HashSet, ffi::OsString, io::Read, path::PathBuf};

    const INPUT_LIMIT: usize = 256 * 1024;

    #[derive(Debug, PartialEq, Eq)]
    pub enum RunError {
        Usage,
        Input,
        Artifact,
        Tokenizer,
        Encoder,
        Output,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Corpus {
        corpus_id: String,
        batches: Vec<Batch>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Batch {
        id: String,
        texts: Vec<String>,
    }

    fn valid_id(id: &str) -> bool {
        (1..=64).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }

    fn read_corpus(reader: impl Read) -> Result<(Corpus, String), RunError> {
        let mut bytes = Vec::new();
        reader
            .take((INPUT_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| RunError::Input)?;
        if bytes.len() > INPUT_LIMIT {
            return Err(RunError::Input);
        }
        let corpus: Corpus = serde_json::from_slice(&bytes).map_err(|_| RunError::Input)?;
        if !valid_id(&corpus.corpus_id)
            || !corpus.corpus_id.starts_with("c0")
            || !(1..=32).contains(&corpus.batches.len())
        {
            return Err(RunError::Input);
        }
        let mut ids = HashSet::new();
        for batch in &corpus.batches {
            if !valid_id(&batch.id)
                || !ids.insert(&batch.id)
                || !(1..=32).contains(&batch.texts.len())
                || batch.texts.iter().any(|text| text.len() > 4096)
            {
                return Err(RunError::Input);
            }
        }
        Ok((corpus, format!("{:x}", Sha256::digest(&bytes))))
    }

    fn parse(
        mut arguments: impl Iterator<Item = OsString>,
    ) -> Result<(PathBuf, String, PathBuf), RunError> {
        let root = PathBuf::from(arguments.next().ok_or(RunError::Usage)?);
        let bundle = arguments
            .next()
            .ok_or(RunError::Usage)?
            .into_string()
            .map_err(|_| RunError::Usage)?;
        let input = PathBuf::from(arguments.next().ok_or(RunError::Usage)?);
        if !root.is_absolute()
            || arguments.next().is_some()
            || bundle.len() != 64
            || !bundle
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(RunError::Usage);
        }
        Ok((root, bundle, input))
    }

    #[derive(Serialize)]
    struct BatchExport {
        id: String,
        input_ids: Vec<Vec<u32>>,
        attention_mask: Vec<Vec<u32>>,
        token_type_ids: Vec<Vec<u32>>,
        vectors: Vec<Vec<f32>>,
    }
    #[derive(Serialize)]
    struct Export {
        schema_version: u32,
        corpus_id: String,
        input_sha256: String,
        artifact_bundle_id: String,
        execution: &'static str,
        token_source: &'static str,
        batches: Vec<BatchExport>,
    }

    fn validate_vectors(vectors: &[Vec<f32>], count: usize) -> Result<(), RunError> {
        if vectors.len() != count
            || vectors
                .iter()
                .any(|row| row.len() != 384 || row.iter().any(|v| !v.is_finite()))
        {
            return Err(RunError::Output);
        }
        Ok(())
    }

    pub fn run() -> Result<(), RunError> {
        let (root, bundle, input) = parse(std::env::args_os().skip(1))?;
        // Reject the complete input before opening the artifact root or loading weights.
        let file = std::fs::File::open(input).map_err(|_| RunError::Input)?;
        if !file.metadata().map_err(|_| RunError::Input)?.is_file() {
            return Err(RunError::Input);
        }
        let (corpus, input_sha256) = read_corpus(file)?;
        let root = ArtifactDir::open_root(&root).map_err(|_| RunError::Artifact)?;
        let manifest = PinnedMiniLmManifest::load_pinned(std::io::Cursor::new(include_bytes!(
            "../../../../docs/muninn/evidence/artifact-lock.json"
        )))
        .map_err(|_| RunError::Artifact)?;
        let directory = root.open_subdir(&bundle).map_err(|_| RunError::Artifact)?;
        let cache =
            VerifiedArtifactCache::open(directory, manifest).map_err(|_| RunError::Artifact)?;
        if cache.bundle_id() != bundle {
            return Err(RunError::Artifact);
        }
        let tokenizer = MiniLmTokenizer::from_cache(&cache).map_err(|_| RunError::Tokenizer)?;
        let encoder = CandidateCpuSentenceEncoder::load(&cache).map_err(|_| RunError::Encoder)?;
        let mut batches = Vec::with_capacity(corpus.batches.len());
        for batch in corpus.batches {
            let texts: Vec<&str> = batch.texts.iter().map(String::as_str).collect();
            // Recomputed independently from the same verified cache; not an internal capture.
            let tokens = tokenizer.encode(&texts).map_err(|_| RunError::Tokenizer)?;
            let vectors = encoder.encode(&texts).map_err(|_| RunError::Encoder)?;
            validate_vectors(&vectors, batch.texts.len())?;
            batches.push(BatchExport {
                id: batch.id,
                input_ids: tokens.input_ids().to_vec(),
                attention_mask: tokens.attention_mask().to_vec(),
                token_type_ids: tokens.token_type_ids().to_vec(),
                vectors,
            });
        }
        let export = Export {
            schema_version: 1,
            corpus_id: corpus.corpus_id,
            input_sha256,
            artifact_bundle_id: cache.bundle_id().to_owned(),
            execution: "candidate_cpu_f32",
            token_source: "independently_recomputed_same_verified_cache",
            batches,
        };
        // Serialize completely before emitting any payload. An OS write failure can still
        // interrupt stdout; consumers must require successful exit and complete JSON.
        let output = serde_json::to_vec(&export).map_err(|_| RunError::Output)?;
        std::io::stdout()
            .lock()
            .write_all(&output)
            .map_err(|_| RunError::Output)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;
        use sha2::{Digest, Sha256};

        fn input(id: &str, batches: usize, texts: Vec<String>) -> Vec<u8> {
            serde_json::to_vec(&json!({"corpus_id":id,"batches":(0..batches).map(|i|json!({"id":format!("b_{i}"),"texts":texts})).collect::<Vec<_>>()})).unwrap()
        }
        #[test]
        fn accepts_c0_and_hashes_exact_bytes() {
            let bytes = input("c0_test", 1, vec![String::new(), "á".into()]);
            let (corpus, hash) = read_corpus(bytes.as_slice()).unwrap();
            assert_eq!(corpus.corpus_id, "c0_test");
            assert_eq!(hash, format!("{:x}", Sha256::digest(&bytes)));
        }
        #[test]
        fn rejects_unknown_fields_at_both_levels() {
            for bytes in [
                br#"{"corpus_id":"c0","batches":[],"extra":1}"#.as_slice(),
                br#"{"corpus_id":"c0","batches":[{"id":"b","texts":["x"],"extra":1}]}"#,
            ] {
                assert!(read_corpus(bytes).is_err());
            }
        }
        #[test]
        fn validates_ids_and_duplicates() {
            for id in ["", "c1", "c0\n", "c0/abc", "c0á", &"c0".repeat(33)] {
                assert!(read_corpus(input(id, 1, vec!["x".into()]).as_slice()).is_err());
            }
            for id in ["b", "bad\n", "", "é", &"x".repeat(65)] {
                let bytes = serde_json::to_vec(&json!({"corpus_id":"c0","batches":[{"id":"b","texts":[""]},{"id":id,"texts":[""]}]})).unwrap();
                assert!(read_corpus(bytes.as_slice()).is_err());
            }
        }
        #[test]
        fn enforces_batch_text_and_byte_bounds() {
            for batches in [0, 33] {
                assert!(read_corpus(input("c0", batches, vec!["x".into()]).as_slice()).is_err());
            }
            for texts in [vec![], vec!["x".into(); 33], vec!["é".repeat(2049)]] {
                assert!(read_corpus(input("c0", 1, texts).as_slice()).is_err());
            }
            assert!(read_corpus(input("c0", 1, vec!["é".repeat(2048)]).as_slice()).is_ok());
            assert!(read_corpus(input("c0", 32, vec![String::new(); 32]).as_slice()).is_ok());
        }
        #[test]
        fn bounds_reader_before_json_parse() {
            let mut bytes = input("c0", 1, vec!["x".into()]);
            bytes.resize(INPUT_LIMIT, b' ');
            assert!(read_corpus(bytes.as_slice()).is_ok());
            bytes.push(b' ');
            assert!(read_corpus(bytes.as_slice()).is_err());
            assert!(read_corpus(std::io::repeat(b' ')).is_err());
        }
        #[test]
        fn cli_requires_absolute_root_and_exact_arguments() {
            let args = |s: &[&str]| s.iter().map(OsString::from).collect::<Vec<_>>().into_iter();
            assert!(parse(args(&["/private/cache", &"a".repeat(64), "input.json"])).is_ok());
            for values in [
                vec![],
                vec!["relative", "bundle", "input"],
                vec!["/root", "../bundle", "input"],
                vec!["/root", "bundle", "input", "extra"],
            ] {
                assert!(parse(args(&values)).is_err());
            }
        }
        #[test]
        fn validates_shape_and_finite_values_before_serialization() {
            assert!(validate_vectors(&[vec![0.25; 384]], 1).is_ok());
            for vectors in [
                vec![],
                vec![vec![0.0; 383]],
                vec![vec![f32::NAN; 384]],
                vec![vec![f32::INFINITY; 384]],
            ] {
                assert!(validate_vectors(&vectors, 1).is_err());
            }
        }
        #[test]
        fn serializes_version_and_candidate_identity() {
            let export = Export {
                schema_version: 1,
                corpus_id: "c0_test".into(),
                input_sha256: "a".repeat(64),
                artifact_bundle_id: "b".repeat(64),
                execution: "candidate_cpu_f32",
                token_source: "independently_recomputed_same_verified_cache",
                batches: Vec::new(),
            };
            let value = serde_json::to_value(export).unwrap();
            assert_eq!(value["schema_version"], 1);
            assert_eq!(value["execution"], "candidate_cpu_f32");
            assert_eq!(value["input_sha256"], "a".repeat(64));
            assert_eq!(value["artifact_bundle_id"], "b".repeat(64));
            assert!(value.get("verdict").is_none());
        }

        #[test]
        fn serializes_token_arrays_and_f32_vectors_without_text_payload() {
            let batch = BatchExport {
                id: "b".into(),
                input_ids: vec![vec![101, 102]],
                attention_mask: vec![vec![1, 1]],
                token_type_ids: vec![vec![0, 0]],
                vectors: vec![vec![0.25; 384]],
            };
            let value = serde_json::to_value(batch).unwrap();
            assert_eq!(value["input_ids"], json!([[101, 102]]));
            assert_eq!(value["vectors"][0].as_array().unwrap().len(), 384);
            assert!(value.get("texts").is_none());
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn main() {
    if let Err(error) = supported::run() {
        eprintln!("export_minilm_c0: {error:?}");
        std::process::exit(1);
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn main() {
    eprintln!("export_minilm_c0: unsupported platform");
    std::process::exit(1);
}
