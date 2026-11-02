use super::*;

const CONFIG: &[u8] = include_bytes!("../tests/fixtures/minilm-config.json");

#[test]
fn loader_rejects_configuration_before_reading_weights_or_constructing_a_model() {
    use crate::{
        artifact_fs::{
            staging::model::{ModelArtifactStage, VerifiedArtifactCache},
            ArtifactDir,
        },
        model_manifest::PinnedMiniLmManifest,
    };
    use std::{fs, io::Cursor, os::unix::fs::PermissionsExt};
    let root = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(
            std::env::temp_dir()
                .canonicalize()
                .expect("physical parent"),
        )
        .expect("fixture");
    let parent = ArtifactDir::open_root(root.path()).expect("fixture root");
    let manifest = PinnedMiniLmManifest::synthetic_for_tests();
    let mut stage = ModelArtifactStage::new(&parent, PinnedMiniLmManifest::synthetic_for_tests())
        .expect("fixture stage");
    for file in manifest.files() {
        stage
            .stage_artifact(file.path(), Cursor::new(b"abc"))
            .expect("synthetic bytes, no valid BERT configuration");
    }
    let published = stage.publish().expect("complete synthetic cache");
    let bundle = published.bundle_id().to_owned();
    drop(published);
    let cache = VerifiedArtifactCache::open(
        parent.open_subdir(&bundle).expect("fixture bundle"),
        manifest,
    )
    .expect("accepted synthetic integrity fixture");
    let path = root.path().join(&bundle);
    struct FixtureCleanup(std::path::PathBuf);
    impl Drop for FixtureCleanup {
        fn drop(&mut self) {
            let _ =
                fs::set_permissions(self.0.join("1_Pooling"), fs::Permissions::from_mode(0o700));
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }
    let _cleanup = FixtureCleanup(path.clone());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("fixture owner mutation");
    fs::remove_file(path.join("model.safetensors")).expect("remove fixture weights only");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).expect("restore fixture mode");
    let result = CpuCheckpoint::load(&cache);
    // If weights were read first, this would be a cache filesystem error.
    assert!(matches!(result, Err(CheckpointError::Configuration)));
    drop(cache);
}

#[test]
fn rejects_a_valid_safetensors_buffer_above_the_total_byte_limit() {
    // This zero-filled payload is a synthetic U8 tensor, not model weights.
    // Its valid format distinguishes our limit from a later parser rejection.
    let payload = usize::try_from(crate::artifacts::MAX_ARTIFACT_BYTES).expect("host limit");
    let header = serde_json::to_vec(&serde_json::json!({"synthetic": {
        "dtype":"U8", "shape":[payload], "data_offsets":[0,payload]
    }}))
    .expect("synthetic header");
    let mut buffer = vec![0_u8; 8 + header.len() + payload];
    buffer[..8].copy_from_slice(
        &u64::try_from(header.len())
            .expect("header length")
            .to_le_bytes(),
    );
    buffer[8..8 + header.len()].copy_from_slice(&header);
    assert!(safetensors::SafeTensors::deserialize(&buffer).is_ok());
    assert!(matches!(
        contract::tensors(&buffer),
        Err(CheckpointError::TensorFormat)
    ));
}

#[test]
fn accepts_the_complete_pinned_bert_configuration() {
    let config = contract::config(CONFIG);
    assert!(config.is_ok());
    let config = config.expect("validated BERT profile");
    assert_eq!(config.hidden_size, 384);
    assert_eq!(config.num_hidden_layers, 6);
    assert_eq!(
        config.hidden_act,
        candle_transformers::models::bert::HiddenAct::Gelu
    );
    assert_eq!(config.classifier_dropout, None);
    assert_eq!(config.model_type.as_deref(), Some("bert"));
}

#[test]
fn rejects_every_missing_unknown_duplicate_or_changed_configuration_field() {
    let original: serde_json::Value = serde_json::from_slice(CONFIG).expect("fixture");
    let object = original.as_object().expect("fixture object");
    assert_eq!(object.len(), 20);
    for name in object.keys() {
        let mut missing = object.clone();
        missing.remove(name);
        assert!(matches!(
            contract::config(&serde_json::to_vec(&missing).expect("fixture")),
            Err(CheckpointError::Configuration)
        ));
        let mut changed = object.clone();
        let value = changed.get_mut(name).expect("field");
        *value = match value {
            serde_json::Value::String(_) => serde_json::json!("different"),
            serde_json::Value::Bool(value) => serde_json::json!(!*value),
            serde_json::Value::Array(_) => serde_json::json!(["WrongModel"]),
            _ => serde_json::json!(99),
        };
        assert!(matches!(
            contract::config(&serde_json::to_vec(&changed).expect("fixture")),
            Err(CheckpointError::Configuration)
        ));
    }
    let duplicate =
        String::from_utf8(CONFIG.to_vec())
            .expect("UTF8")
            .replacen('{', "{\"hidden_size\":384,", 1);
    assert!(contract::config(duplicate.as_bytes()).is_err());
    let mut unknown = object.clone();
    unknown.insert("trust_remote_code".into(), serde_json::json!(true));
    assert!(contract::config(&serde_json::to_vec(&unknown).expect("fixture")).is_err());
    for bytes in [b"{}".as_slice(), b"null", &[b' '; 16 * 1024 + 1]] {
        assert!(contract::config(bytes).is_err());
    }
}

#[test]
fn validates_original_little_endian_f32_values_without_casting() {
    let valid: Vec<_> = [0_f32, -0_f32, 1.5, f32::MIN, f32::MAX]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    assert_eq!(contract::finite_f32(&valid), Ok(()));
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            contract::finite_f32(&invalid.to_le_bytes()),
            Err(CheckpointError::TensorValues)
        );
    }
    assert_eq!(
        contract::finite_f32(&[1, 2, 3]),
        Err(CheckpointError::TensorValues)
    );
}

#[test]
fn specifies_all_104_unprefixed_tensor_shapes_and_original_dtypes() {
    let specs = contract::specifications();
    assert_eq!(specs.len(), 104);
    assert_eq!(
        specs
            .values()
            .filter(|(dtype, _)| *dtype == safetensors::Dtype::F32)
            .count(),
        103
    );
    assert_eq!(
        specs["embeddings.word_embeddings.weight"],
        (safetensors::Dtype::F32, vec![30522, 384])
    );
    assert_eq!(
        specs["embeddings.position_ids"],
        (safetensors::Dtype::I64, vec![1, 512])
    );
    assert_eq!(specs["pooler.dense.weight"].1, vec![384, 384]);
    for layer in 0..6 {
        let prefix = format!("encoder.layer.{layer}.");
        assert_eq!(
            specs
                .keys()
                .filter(|name| name.starts_with(&prefix))
                .count(),
            16
        );
        assert_eq!(
            specs[&format!("{prefix}intermediate.dense.weight")].1,
            vec![1536, 384]
        );
        assert_eq!(
            specs[&format!("{prefix}output.dense.weight")].1,
            vec![384, 1536]
        );
        for part in ["query", "key", "value"] {
            assert_eq!(
                specs[&format!("{prefix}attention.self.{part}.weight")].1,
                vec![384, 384]
            );
            assert_eq!(
                specs[&format!("{prefix}attention.self.{part}.bias")].1,
                vec![384]
            );
        }
    }
    assert!(specs.keys().all(|name| !name.starts_with("bert.")));
}

#[test]
fn validates_the_i64_position_buffer_exactly_without_float_casting() {
    use safetensors::{tensor::TensorView, Dtype};
    let specs = contract::specifications();
    let expected = &specs["embeddings.position_ids"];
    let mut data: Vec<_> = (0_i64..512).flat_map(i64::to_le_bytes).collect();
    let view = TensorView::new(Dtype::I64, vec![1, 512], &data).expect("integer fixture");
    assert_eq!(contract::tensor(expected, &view), Ok(()));
    data[8..16].copy_from_slice(&(-1_i64).to_le_bytes());
    let bad = TensorView::new(Dtype::I64, vec![1, 512], &data).expect("bad values fixture");
    assert_eq!(
        contract::tensor(expected, &bad),
        Err(CheckpointError::TensorValues)
    );
}

#[test]
fn uses_the_safe_parser_with_tighter_header_and_total_bounds() {
    use safetensors::{
        tensor::{serialize, TensorView},
        Dtype,
    };
    let data = 1_f32.to_le_bytes();
    let view = TensorView::new(Dtype::F32, vec![1], &data).expect("small synthetic view");
    let bytes = serialize([("synthetic", view)], None).expect("synthetic safetensors");
    assert!(contract::tensors(&bytes).is_ok());
    for invalid in [
        vec![],
        vec![0; 7],
        (1024_u64 * 1024 + 1).to_le_bytes().to_vec(),
        u64::MAX.to_le_bytes().to_vec(),
        bytes[..bytes.len() - 1].to_vec(),
    ] {
        assert!(matches!(
            contract::tensors(&invalid),
            Err(CheckpointError::TensorFormat)
        ));
    }
}

#[test]
fn requires_every_expected_name_without_prefix_aliases_or_additional_tensors() {
    let specs = contract::specifications();
    let names: Vec<_> = specs.keys().map(String::as_str).collect();
    assert!(contract::inventory(names.iter().copied()).is_ok());
    assert!(contract::inventory(names.iter().skip(1).copied()).is_err());
    let mut replaced = names.clone();
    replaced[0] = "bert.embeddings.LayerNorm.bias";
    assert!(contract::inventory(replaced.iter().copied()).is_err());
    let mut extra = names.clone();
    extra.push("unexpected");
    assert!(contract::inventory(extra.iter().copied()).is_err());
    let mut duplicate = names.clone();
    duplicate[0] = duplicate[1];
    assert!(contract::inventory(duplicate.iter().copied()).is_err());
}

#[test]
fn original_dtype_and_shape_must_match_before_any_conversion() {
    use safetensors::{tensor::TensorView, Dtype};
    let specs = contract::specifications();
    let expected = &specs["embeddings.LayerNorm.bias"];
    let f32_bytes = vec![0; 384 * 4];
    let valid = TensorView::new(Dtype::F32, vec![384], &f32_bytes).expect("valid float view");
    assert_eq!(contract::tensor(expected, &valid), Ok(()));
    let half_bytes = vec![0; 384 * 2];
    let half = TensorView::new(Dtype::F16, vec![384], &half_bytes).expect("unsupported float view");
    assert_eq!(
        contract::tensor(expected, &half),
        Err(CheckpointError::TensorDtype)
    );
    let reshaped = TensorView::new(Dtype::F32, vec![1, 384], &f32_bytes).expect("wrong shape");
    assert_eq!(
        contract::tensor(expected, &reshaped),
        Err(CheckpointError::TensorShape)
    );
}

#[test]
#[ignore = "Requires the operator-supplied verified actual MiniLM cache; no network or model construction"]
fn actual_cache_contract_validates_all_original_tensors_without_model_construction() {
    let cache = actual_cache();
    let report = validate(&cache).expect("strict original checkpoint contract");
    assert_eq!(
        report,
        CheckpointReport {
            tensor_entries: 104,
            original_f32_entries: 103,
            position_ids_checked: 512,
            weight_bytes: 90_868_376
        }
    );
    println!("{}", serde_json::to_string(&report).expect("report"));
}

fn actual_cache() -> crate::artifact_fs::staging::model::VerifiedArtifactCache {
    use crate::{
        artifact_fs::{staging::model::VerifiedArtifactCache, ArtifactDir},
        model_manifest::PinnedMiniLmManifest,
    };
    let root =
        std::env::var_os("MUNINN_MINILM_ARTIFACT_ROOT").expect("explicit operator cache root");
    let bundle = "ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1";
    let parent = ArtifactDir::open_root(std::path::Path::new(&root)).expect("safe existing root");
    let manifest = PinnedMiniLmManifest::load_pinned(std::io::Cursor::new(include_bytes!(
        "../../../../docs/muninn/evidence/artifact-lock.json"
    )))
    .expect("compiled pin");
    VerifiedArtifactCache::open(parent.open_subdir(bundle).expect("bundle"), manifest)
        .expect("verified all actual artifacts")
}

#[test]
#[ignore = "Explicit real CPU BERT construction only; no forward pass, tokenizer or numerical acceptance"]
fn actual_cache_constructs_cpu_bert_from_validated_owned_bytes() {
    let cache = actual_cache();
    let checkpoint = CpuCheckpoint::load(&cache);
    assert!(checkpoint.is_ok());
    let checkpoint = checkpoint.expect("constructed real CPU BERT");
    assert!(checkpoint.model().device.is_cpu());
    assert_eq!(
        checkpoint.report(),
        &CheckpointReport {
            tensor_entries: 104,
            original_f32_entries: 103,
            position_ids_checked: 512,
            weight_bytes: 90_868_376
        }
    );
    println!(
        "{}",
        serde_json::json!({"constructed":"BertModel", "device":"Cpu", "construction_dtype":"F32",
        "contract":checkpoint.report(), "forward_executed":false, "inference_executed":false})
    );
}
