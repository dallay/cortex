use super::*;

fn synthetic() -> Tokenizer {
    from_synthetic_source(synthetic_source())
}

fn synthetic_source() -> serde_json::Value {
    serde_json::json!({
        "version":"1.0", "truncation":{"direction":"Right","max_length":128,"strategy":"LongestFirst","stride":0},
        "padding":{"strategy":{"Fixed":128},"direction":"Right","pad_to_multiple_of":null,"pad_id":0,"pad_type_id":0,"pad_token":"[PAD]"},
        "added_tokens":[], "normalizer":{"type":"BertNormalizer","clean_text":true,"handle_chinese_chars":true,"strip_accents":null,"lowercase":true},
        "pre_tokenizer":{"type":"BertPreTokenizer"}, "post_processor":null,
        "decoder":{"type":"WordPiece","prefix":"##","cleanup":true},
        "model":{"type":"WordPiece","unk_token":"[UNK]","continuing_subword_prefix":"##","max_input_chars_per_word":100,
            "vocab":{"[PAD]":0,"[UNK]":100,"[CLS]":101,"[SEP]":102,"[MASK]":103,"hello":7592,"world":2088,"!":999}}
    })
}

fn from_synthetic_source(source: serde_json::Value) -> Tokenizer {
    let mut tokenizer = Tokenizer::from_bytes(serde_json::to_vec(&source).expect("synthetic JSON"))
        .expect("small synthetic WordPiece");
    let processor = tokenizers::processors::template::TemplateProcessing::builder()
        .try_single("[CLS] $A [SEP]")
        .expect("single text template")
        .special_tokens(vec![("[CLS]", 101), ("[SEP]", 102)])
        .build()
        .expect("synthetic processor");
    tokenizer.with_post_processor(Some(processor));
    tokenizer
}

#[test]
fn constructs_a_tokenizer_with_the_declared_special_ids() {
    let result = MiniLmTokenizer::configure(synthetic());
    assert!(result.is_ok());
    assert_eq!(
        result
            .expect("configured tokenizer")
            .tokenizer
            .token_to_id("[CLS]"),
        Some(101)
    );
}

#[test]
fn rejects_mismatched_special_ids_or_missing_postprocessor() {
    let mut source = synthetic_source();
    source["model"]["vocab"]["[CLS]"] = serde_json::json!(104);
    let wrong = from_synthetic_source(source);
    assert_eq!(wrong.token_to_id("[CLS]"), Some(104));
    assert!(wrong.get_post_processor().is_some());
    assert!(matches!(
        MiniLmTokenizer::configure(wrong),
        Err(TokenizerError::Configuration)
    ));
    let mut missing = synthetic();
    missing.with_post_processor(None::<tokenizers::processors::template::TemplateProcessing>);
    assert!(matches!(
        MiniLmTokenizer::configure(missing),
        Err(TokenizerError::Configuration)
    ));
}

#[test]
fn pads_only_to_the_longest_text_including_special_tokens() {
    let tokenizer = MiniLmTokenizer::configure(synthetic()).expect("synthetic tokenizer");
    let batch = tokenizer.encode(&["", "Hello world!"]).expect("text batch");
    assert_eq!(
        batch.input_ids(),
        &[vec![101, 102, 0, 0, 0], vec![101, 7592, 2088, 999, 102]]
    );
    assert_eq!(batch.token_type_ids(), &[vec![0; 5], vec![0; 5]]);
    assert_eq!(batch.attention_mask(), &[vec![1, 1, 0, 0, 0], vec![1; 5]]);
    assert_eq!(
        tokenizer.encode(&[""]).expect("empty text").input_ids(),
        &[vec![101, 102]]
    );
}

#[test]
fn overrides_128_to_256_including_exactly_once_added_specials() {
    let tokenizer = MiniLmTokenizer::configure(synthetic()).expect("synthetic tokenizer");
    let text = "hello ".repeat(300);
    let before = text.clone();
    let batch = tokenizer.encode(&[&text]).expect("long text");
    assert_eq!(batch.input_ids()[0].len(), 256);
    assert_eq!(batch.input_ids()[0][0], 101);
    assert_eq!(batch.input_ids()[0][255], 102);
    assert!(batch.input_ids()[0][1..255].iter().all(|id| *id == 7592));
    assert_eq!(batch.attention_mask()[0], vec![1; 256]);
    assert_eq!(text, before);
}

#[test]
fn rejects_empty_or_over_limit_batches_without_encoding() {
    let tokenizer = MiniLmTokenizer::configure(synthetic()).expect("synthetic tokenizer");
    assert!(matches!(
        tokenizer.encode(&[]),
        Err(TokenizerError::InputLimit)
    ));
    assert!(matches!(
        tokenizer.encode(&[""; MAX_BATCH + 1]),
        Err(TokenizerError::InputLimit)
    ));
    let text = "x".repeat(MAX_INPUT_BYTES + 1);
    assert!(matches!(
        tokenizer.encode(&[&text]),
        Err(TokenizerError::InputLimit)
    ));
    assert!(tokenizer.encode(&[""; MAX_BATCH]).is_ok());
}

fn valid_batch() -> TokenBatch {
    TokenBatch {
        input_ids: vec![vec![101, 102, 0], vec![101, 7592, 102]],
        token_type_ids: vec![vec![0; 3], vec![0; 3]],
        attention_mask: vec![vec![1, 1, 0], vec![1, 1, 1]],
    }
}

#[test]
fn validates_exact_row_count_rectangular_width_and_dynamic_padding() {
    assert!(valid_batch().checked(2).is_ok());
    assert!(valid_batch().checked(1).is_err());
    let mut fixed = valid_batch();
    for row in &mut fixed.input_ids {
        row.push(0);
    }
    for row in &mut fixed.token_type_ids {
        row.push(0);
    }
    for row in &mut fixed.attention_mask {
        row.push(0);
    }
    assert!(fixed.checked(2).is_err());
    let mut ragged = valid_batch();
    ragged.attention_mask[0].pop();
    assert!(ragged.checked(2).is_err());
    let mut missing = valid_batch();
    missing.token_type_ids.pop();
    assert!(missing.checked(2).is_err());
}

#[test]
fn rejects_malformed_token_types_masks_boundaries_and_padding() {
    for change in 0..7 {
        let mut batch = valid_batch();
        match change {
            0 => batch.input_ids[0][0] = 0,
            1 => batch.input_ids[0][1] = 7592,
            2 => batch.input_ids[0][2] = 7592,
            3 => batch.token_type_ids[0][0] = 1,
            4 => batch.attention_mask[0] = vec![1, 0, 1],
            5 => batch.attention_mask[0][1] = 2,
            _ => batch.input_ids[1][1] = 30522,
        }
        assert!(batch.checked(2).is_err());
    }
    // A literal [PAD] in real input is not padding when its attention mask is one.
    let mut literal = valid_batch();
    literal.input_ids[1][1] = 0;
    assert!(literal.checked(2).is_ok());
}

#[derive(serde::Deserialize)]
struct ReferenceBatch {
    id: String,
    texts: Vec<String>,
    input_ids: Vec<Vec<u32>>,
    token_type_ids: Vec<Vec<u32>>,
    attention_mask: Vec<Vec<u32>>,
}

#[test]
#[ignore = "Requires explicit verified actual cache; exact tokenizer differential only, no model forward"]
fn actual_cache_matches_pinned_python_ids_types_and_masks_exactly() {
    use crate::{
        artifact_fs::{staging::model::VerifiedArtifactCache, ArtifactDir},
        model_manifest::PinnedMiniLmManifest,
    };
    let root =
        std::env::var_os("MUNINN_MINILM_ARTIFACT_ROOT").expect("explicit existing cache root");
    let parent = ArtifactDir::open_root(std::path::Path::new(&root)).expect("safe root");
    let manifest = PinnedMiniLmManifest::load_pinned(std::io::Cursor::new(include_bytes!(
        "../../../../docs/muninn/evidence/artifact-lock.json"
    )))
    .expect("compiled pin");
    let cache = VerifiedArtifactCache::open(
        parent
            .open_subdir("ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1")
            .expect("bundle"),
        manifest,
    )
    .expect("actual cache verified");
    let tokenizer = MiniLmTokenizer::from_cache(&cache);
    assert!(tokenizer.is_ok());
    let tokenizer = tokenizer.expect("pinned tokenizer");
    let reference: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../tests/fixtures/minilm-tokenizer-reference.json"
    ))
    .expect("recorded Python fixture");
    use sha2::{Digest, Sha256};
    for (key, bytes) in [
        (
            "artifact_lock_sha256",
            include_bytes!("../../../../docs/muninn/evidence/artifact-lock.json").as_slice(),
        ),
        (
            "reference_requirements_sha256",
            include_bytes!("../../requirements-minilm-tokenizer.txt").as_slice(),
        ),
        (
            "generator_sha256",
            include_bytes!("../../minilm_tokenizer_reference.py").as_slice(),
        ),
    ] {
        assert_eq!(reference[key], format!("{:x}", Sha256::digest(bytes)));
    }
    assert_eq!(reference["packages"]["tokenizers"], "0.23.3");
    assert_eq!(reference["packages"]["transformers"], "5.20.2");
    assert_eq!(
        reference["reference_commit"],
        "9d45af5529ecb5f2fe352013468a11087f3163d4"
    );
    assert_eq!(reference["max_length_including_specials"], 256);
    assert_eq!(reference["tokenizer_class"], "BertTokenizer");
    assert_eq!(reference["fast_backend"], true);
    assert_eq!(
        reference["effective_settings"],
        serde_json::json!({"add_special_tokens":true,"padding":"longest","padding_side":"right",
        "truncation":"longest_first","truncation_side":"right","stride":0,"max_length":256})
    );
    assert_eq!(reference["model_forward_executed"], false);
    let identity = PinnedMiniLmManifest::load_pinned(std::io::Cursor::new(include_bytes!(
        "../../../../docs/muninn/evidence/artifact-lock.json"
    )))
    .expect("pin for fixture provenance");
    assert_eq!(reference["revision"], identity.revision());
    let artifact_metadata = reference["artifacts"]
        .as_object()
        .expect("artifact provenance");
    let actual_keys: std::collections::BTreeSet<_> =
        artifact_metadata.keys().map(String::as_str).collect();
    assert_eq!(
        actual_keys,
        std::collections::BTreeSet::from([
            "config.json",
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
            "sentence_bert_config.json",
            "vocab.txt"
        ])
    );
    for (name, metadata) in artifact_metadata {
        let artifact = identity
            .files()
            .iter()
            .find(|file| file.path() == name)
            .expect("declared artifact");
        assert_eq!(metadata["sha256"], artifact.sha256());
        assert_eq!(metadata["bytes"], artifact.size());
    }
    let cases: Vec<ReferenceBatch> =
        serde_json::from_value(reference["batches"].clone()).expect("fixture cases");
    let case_ids: Vec<_> = cases.iter().map(|case| case.id.as_str()).collect();
    assert_eq!(
        case_ids,
        [
            "empty",
            "basic",
            "unicode",
            "controls_and_whitespace",
            "literal_specials",
            "content_126",
            "content_254",
            "content_255",
            "content_500",
            "mixed"
        ]
    );
    assert_eq!(cases.iter().map(|case| case.texts.len()).sum::<usize>(), 19);
    let mut texts_checked = 0;
    for case in &cases {
        let input: Vec<_> = case.texts.iter().map(String::as_str).collect();
        let encoded = tokenizer.encode(&input).expect("bounded batch");
        assert_eq!(encoded.input_ids(), case.input_ids, "IDs case {}", case.id);
        assert_eq!(
            encoded.token_type_ids(),
            case.token_type_ids,
            "types case {}",
            case.id
        );
        assert_eq!(
            encoded.attention_mask(),
            case.attention_mask,
            "masks case {}",
            case.id
        );
        for (index, text) in input.iter().enumerate() {
            let single = tokenizer.encode(&[*text]).expect("single text from batch");
            let real = case.attention_mask[index]
                .iter()
                .filter(|value| **value == 1)
                .count();
            assert_eq!(
                single.input_ids()[0],
                case.input_ids[index][..real],
                "single/batch IDs case {}",
                case.id
            );
            assert_eq!(
                single.token_type_ids()[0],
                case.token_type_ids[index][..real]
            );
            assert_eq!(single.attention_mask()[0], vec![1; real]);
        }
        texts_checked += input.len();
    }
    println!(
        "{}",
        serde_json::json!({"batches_exact":cases.len(),"texts_exact":texts_checked,
        "rust_tokenizers":"0.22.2","python_tokenizers":"0.23.3","model_forward_executed":false})
    );
}
