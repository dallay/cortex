//! Selección del manifiesto MiniLM aprobado; no descarga ni carga modelos.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::Read};

use crate::artifacts::{
    is_lower_hex, load_verified, validate_spec, ArtifactError, VerifiedArtifact,
};

const PINNED_LOCK: &[u8] = include_bytes!("../../../../dev/muninn/fixtures/artifact-lock.json");
pub(crate) const MODEL_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";
const REQUIRED_FILES: &[&str] = &[
    "config.json",
    "config_sentence_transformers.json",
    "modules.json",
    "1_Pooling/config.json",
    "sentence_bert_config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
    "vocab.txt",
    "README.md",
    "model.safetensors",
];

#[derive(Debug, PartialEq, Eq)]
pub enum ManifestError {
    Artifact(ArtifactError),
    InvalidManifest,
    UnknownArtifact,
}

pub struct PinnedMiniLmManifest {
    revision: String,
    files: Vec<ModelArtifact>,
}

pub struct ModelArtifact {
    path: String,
    size: u64,
    sha256: String,
    source_url: String,
}

#[derive(Deserialize)]
struct ModelRecord {
    source: String,
    revision: String,
    files: Vec<RawArtifact>,
}

#[derive(Deserialize)]
struct RawArtifact {
    path: String,
    size: u64,
    sha256: String,
    source_url: String,
}

impl PinnedMiniLmManifest {
    #[cfg(test)]
    pub(crate) fn synthetic_oversized_for_tests() -> Self {
        let mut manifest = Self::synthetic_for_tests();
        for file in &mut manifest.files {
            file.size = crate::artifacts::MAX_ARTIFACT_BYTES;
        }
        manifest
    }

    #[cfg(test)]
    pub(crate) fn synthetic_for_tests() -> Self {
        let mut root: serde_json::Value = serde_json::from_slice(PINNED_LOCK).expect("fixture");
        let model = root["models"]
            .as_array_mut()
            .expect("models")
            .iter_mut()
            .find(|model| model["id"] == MODEL_ID)
            .expect("MiniLM");
        for file in model["files"].as_array_mut().expect("files") {
            file["size"] = serde_json::json!(3);
            file["sha256"] = serde_json::json!(
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            );
        }
        Self::parse(&serde_json::to_vec(&root).expect("fixture JSON"))
            .expect("synthetic private fixture")
    }

    pub fn load_pinned<R: Read>(reader: R) -> Result<Self, ManifestError> {
        let checked = load_verified(
            reader,
            u64::try_from(PINNED_LOCK.len()).map_err(|_| ManifestError::InvalidManifest)?,
            &format!("{:x}", Sha256::digest(PINNED_LOCK)),
        )
        .map_err(ManifestError::Artifact)?;
        Self::parse(checked.as_bytes())
    }

    fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        let root: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| ManifestError::InvalidManifest)?;
        if root
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
        {
            return Err(ManifestError::InvalidManifest);
        }
        let mut models = root
            .get("models")
            .and_then(serde_json::Value::as_array)
            .ok_or(ManifestError::InvalidManifest)?
            .iter()
            .filter(|model| model.get("id").and_then(serde_json::Value::as_str) == Some(MODEL_ID));
        let model = models.next().ok_or(ManifestError::InvalidManifest)?;
        if models.next().is_some() {
            return Err(ManifestError::InvalidManifest);
        }
        let model: ModelRecord =
            serde_json::from_value(model.clone()).map_err(|_| ManifestError::InvalidManifest)?;
        if model.source != "huggingface" || !is_lower_hex(&model.revision, 40) {
            return Err(ManifestError::InvalidManifest);
        }
        if model.files.len() != REQUIRED_FILES.len()
            || model
                .files
                .iter()
                .any(|file| !REQUIRED_FILES.contains(&file.path.as_str()))
            || model
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                != REQUIRED_FILES.len()
        {
            return Err(ManifestError::InvalidManifest);
        }
        if model.files.iter().any(|file| {
            validate_spec(file.size, &file.sha256).is_err()
                || file.source_url
                    != format!(
                        "https://huggingface.co/{MODEL_ID}/resolve/{}/{}",
                        model.revision, file.path
                    )
        }) {
            return Err(ManifestError::InvalidManifest);
        }
        let files = model
            .files
            .into_iter()
            .map(|file| ModelArtifact {
                path: file.path,
                size: file.size,
                sha256: file.sha256,
                source_url: file.source_url,
            })
            .collect();
        Ok(Self {
            revision: model.revision,
            files,
        })
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn files(&self) -> &[ModelArtifact] {
        &self.files
    }

    pub fn verify_artifact<R: Read>(
        &self,
        path: &str,
        reader: R,
    ) -> Result<VerifiedArtifact, ManifestError> {
        let artifact = self
            .files
            .iter()
            .find(|file| file.path == path)
            .ok_or(ManifestError::UnknownArtifact)?;
        load_verified(reader, artifact.size, &artifact.sha256).map_err(ManifestError::Artifact)
    }
}

impl ModelArtifact {
    pub fn path(&self) -> &str {
        &self.path
    }
    pub const fn size(&self) -> u64 {
        self.size
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn source_url(&self) -> &str {
        &self.source_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn accepts_the_reviewed_lock_and_selects_only_minilm() {
        let manifest = PinnedMiniLmManifest::load_pinned(Cursor::new(PINNED_LOCK));
        assert!(manifest.is_ok());
        let manifest = manifest.expect("reviewed fixture");
        assert_eq!(
            manifest.revision(),
            "1110a243fdf4706b3f48f1d95db1a4f5529b4d41"
        );
        assert_eq!(manifest.files().len(), 11);
        let weight = manifest
            .files()
            .iter()
            .find(|file| file.path() == "model.safetensors")
            .expect("declared weight");
        assert_eq!(weight.size(), 90_868_376);
        assert_eq!(weight.sha256().len(), 64);
        assert!(weight.source_url().contains(manifest.revision()));
    }

    #[test]
    fn rejects_same_size_manifest_changes_before_parsing() {
        let mut changed = PINNED_LOCK.to_vec();
        changed[0] = b'[';
        assert!(matches!(
            PinnedMiniLmManifest::load_pinned(Cursor::new(changed)),
            Err(ManifestError::Artifact(ArtifactError::DigestMismatch))
        ));
    }

    fn fixture() -> serde_json::Value {
        serde_json::from_slice(PINNED_LOCK).expect("reviewed fixture")
    }

    fn parse_value(root: &serde_json::Value) -> Result<PinnedMiniLmManifest, ManifestError> {
        PinnedMiniLmManifest::parse(&serde_json::to_vec(root).expect("fixture JSON"))
    }

    #[test]
    fn rejects_an_unsupported_manifest_schema() {
        let mut root = fixture();
        root["schema_version"] = serde_json::json!(2);
        assert!(matches!(
            parse_value(&root),
            Err(ManifestError::InvalidManifest)
        ));
    }

    #[test]
    fn rejects_missing_or_ambiguous_minilm_records() {
        let mut root = fixture();
        let models = root["models"].as_array_mut().expect("models");
        let target = models
            .iter()
            .find(|model| model["id"] == MODEL_ID)
            .expect("MiniLM")
            .clone();
        models.push(target);
        assert!(matches!(
            parse_value(&root),
            Err(ManifestError::InvalidManifest)
        ));
        root["models"]
            .as_array_mut()
            .expect("models")
            .retain(|model| model["id"] != MODEL_ID);
        assert!(matches!(
            parse_value(&root),
            Err(ManifestError::InvalidManifest)
        ));
    }

    fn model_mut(root: &mut serde_json::Value) -> &mut serde_json::Value {
        root["models"]
            .as_array_mut()
            .expect("models")
            .iter_mut()
            .find(|model| model["id"] == MODEL_ID)
            .expect("MiniLM")
    }

    #[test]
    fn requires_an_immutable_huggingface_revision() {
        for (field, value) in [
            ("revision", serde_json::json!("main")),
            ("revision", serde_json::json!("A".repeat(40))),
            ("source", serde_json::json!("untrusted-source")),
        ] {
            let mut root = fixture();
            model_mut(&mut root)[field] = value;
            assert!(matches!(
                parse_value(&root),
                Err(ManifestError::InvalidManifest)
            ));
        }
    }

    #[test]
    fn rejects_open_missing_or_duplicate_file_inventories() {
        let mut missing = fixture();
        model_mut(&mut missing)["files"]
            .as_array_mut()
            .expect("files")
            .pop();
        assert!(matches!(
            parse_value(&missing),
            Err(ManifestError::InvalidManifest)
        ));
        let mut duplicate = fixture();
        let files = model_mut(&mut duplicate)["files"]
            .as_array_mut()
            .expect("files");
        files.push(files[0].clone());
        assert!(matches!(
            parse_value(&duplicate),
            Err(ManifestError::InvalidManifest)
        ));
        for path in [
            "../config.json",
            "/config.json",
            "a\\config.json",
            "remote.py",
        ] {
            let mut root = fixture();
            let model = model_mut(&mut root);
            let revision = model["revision"].as_str().expect("revision").to_owned();
            model["files"][0]["path"] = serde_json::json!(path);
            model["files"][0]["source_url"] = serde_json::json!(format!(
                "https://huggingface.co/{MODEL_ID}/resolve/{revision}/{path}"
            ));
            assert!(matches!(
                parse_value(&root),
                Err(ManifestError::InvalidManifest)
            ));
        }
    }

    #[test]
    fn rejects_urls_outside_the_exact_pinned_huggingface_path() {
        let mut original = fixture();
        let url = model_mut(&mut original)["files"][0]["source_url"]
            .as_str()
            .expect("URL")
            .to_owned();
        for changed in [
            "https://evil.invalid/config.json".to_owned(),
            url.replacen("https://", "http://", 1),
            url.replacen("https://", "https://user:password@", 1),
            format!("{url}?extra=1"),
            format!("{url}#fragment"),
            url.replace(
                "/resolve/1110a243fdf4706b3f48f1d95db1a4f5529b4d41/",
                "/resolve/main/",
            ),
        ] {
            let mut root = fixture();
            model_mut(&mut root)["files"][0]["source_url"] = serde_json::json!(changed);
            assert!(matches!(
                parse_value(&root),
                Err(ManifestError::InvalidManifest)
            ));
        }
    }

    #[test]
    fn rejects_invalid_artifact_sizes_and_digests() {
        for (field, value) in [
            ("size", serde_json::json!(0)),
            (
                "size",
                serde_json::json!(crate::artifacts::MAX_ARTIFACT_BYTES + 1),
            ),
            ("size", serde_json::json!(true)),
            ("sha256", serde_json::json!("A".repeat(64))),
            ("sha256", serde_json::json!("a".repeat(63))),
        ] {
            let mut root = fixture();
            model_mut(&mut root)["files"][0][field] = value;
            assert!(matches!(
                parse_value(&root),
                Err(ManifestError::InvalidManifest)
            ));
        }
    }

    fn manifest_with_tiny_file() -> PinnedMiniLmManifest {
        let mut root = fixture();
        let file = &mut model_mut(&mut root)["files"][0];
        file["size"] = serde_json::json!(3);
        file["sha256"] =
            serde_json::json!("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        parse_value(&root).expect("private schema fixture, not a runtime pin override")
    }

    #[test]
    fn verifies_a_listed_file_before_returning_its_buffer() {
        let manifest = manifest_with_tiny_file();
        let buffer = manifest.verify_artifact("config.json", Cursor::new(b"abc"));
        assert!(buffer.is_ok());
        assert_eq!(buffer.expect("verified fixture").as_bytes(), b"abc");
    }

    #[test]
    fn rejects_corrupt_or_truncated_listed_files() {
        let manifest = manifest_with_tiny_file();
        assert!(matches!(
            manifest.verify_artifact("config.json", Cursor::new(b"abd")),
            Err(ManifestError::Artifact(ArtifactError::DigestMismatch))
        ));
        assert!(matches!(
            manifest.verify_artifact("config.json", Cursor::new(b"ab")),
            Err(ManifestError::Artifact(ArtifactError::SizeMismatch))
        ));
    }

    #[test]
    fn rejects_unlisted_files_before_reading() {
        struct Unreadable;
        impl Read for Unreadable {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("must not read an unlisted file"))
            }
        }
        let manifest = manifest_with_tiny_file();
        assert!(matches!(
            manifest.verify_artifact("../unlisted", Unreadable),
            Err(ManifestError::UnknownArtifact)
        ));
    }

    #[test]
    fn rejects_a_truncated_or_extended_lock() {
        assert!(matches!(
            PinnedMiniLmManifest::load_pinned(Cursor::new(&PINNED_LOCK[..PINNED_LOCK.len() - 1])),
            Err(ManifestError::Artifact(ArtifactError::SizeMismatch))
        ));
        let mut extended = PINNED_LOCK.to_vec();
        extended.push(b' ');
        assert!(matches!(
            PinnedMiniLmManifest::load_pinned(Cursor::new(extended)),
            Err(ManifestError::Artifact(ArtifactError::SizeMismatch))
        ));
    }
}
