use super::*;
use candle_core::{DType, Device, Tensor};

fn hidden_fixture() -> Tensor {
    let mut values = vec![0_f32; 3 * 384];
    values[0] = 3.;
    values[384] = 3.;
    values[385] = 8.;
    values[768] = 1e6;
    values[769] = 1e6;
    Tensor::from_vec(values, (1, 3, 384), &Device::Cpu).expect("synthetic hidden tensor")
}

#[test]
fn masked_mean_includes_both_real_special_positions_and_excludes_padding() {
    let mask = Tensor::new(&[[1_u32, 1, 0]], &Device::Cpu).expect("synthetic mask");
    let pooled = math::masked_mean(&hidden_fixture(), &mask);
    assert!(pooled.is_ok());
    let pooled = pooled.expect("masked mean");
    assert_eq!(pooled.dtype(), DType::F32);
    assert_eq!(pooled.dims(), &[1, 384]);
    let values = pooled.to_vec2::<f32>().expect("values");
    assert_eq!(
        [values[0][0].to_bits(), values[0][1].to_bits()],
        [3_f32.to_bits(), 4_f32.to_bits()]
    );
    assert!(values[0][2..].iter().all(|value| *value == 0.));
}

fn row(first: f32, second: f32) -> Tensor {
    let mut values = vec![0_f32; 384];
    values[0] = first;
    values[1] = second;
    Tensor::from_vec(values, (1, 384), &Device::Cpu).expect("synthetic pooled row")
}

#[test]
fn normalizes_after_pooling_without_normalizing_individual_tokens() {
    let mask = Tensor::new(&[[1_u32, 1, 0]], &Device::Cpu).expect("mask");
    let pooled = math::masked_mean(&hidden_fixture(), &mask).expect("mean3,4");
    let normalized = math::normalize(&pooled);
    assert!(normalized.is_ok());
    let values = normalized
        .expect("normalized row")
        .to_vec2::<f32>()
        .expect("F32 values");
    assert_eq!(
        [values[0][0].to_bits(), values[0][1].to_bits()],
        [0.6_f32.to_bits(), 0.8_f32.to_bits()]
    );
    assert!(values[0][2..].iter().all(|value| *value == 0.));
}

#[test]
fn rejects_zero_tiny_nonfinite_and_overflowed_norms_without_zero_fallback() {
    for value in [
        0_f32,
        0.5e-12,
        1e-12,
        1e-30,
        f32::MAX,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ] {
        assert!(matches!(
            math::normalize(&row(value, 0.)),
            Err(EncoderError::Values)
        ));
    }
    assert!(math::normalize(&row(2e-12, 0.)).is_ok());
    assert!(math::normalize(&row(3., 4.)).is_ok());
}

#[test]
fn rejects_wrong_shapes_precision_and_gradients_before_math() {
    let mask = Tensor::new(&[[1_u32, 1, 0]], &Device::Cpu).expect("mask");
    let bad = Tensor::zeros((1, 3, 383), DType::F32, &Device::Cpu).expect("wrong shape");
    assert!(matches!(
        math::masked_mean(&bad, &mask),
        Err(EncoderError::Shape)
    ));
    let double = hidden_fixture()
        .to_dtype(DType::F64)
        .expect("unsupported dtype");
    assert!(matches!(
        math::masked_mean(&double, &mask),
        Err(EncoderError::Precision)
    ));
    let wrong_mask = mask.to_dtype(DType::F32).expect("unsupported mask dtype");
    assert!(matches!(
        math::masked_mean(&hidden_fixture(), &wrong_mask),
        Err(EncoderError::Precision)
    ));
    let wrong_mask = Tensor::new(&[[1_u32, 1]], &Device::Cpu).expect("wrong mask shape");
    assert!(matches!(
        math::masked_mean(&hidden_fixture(), &wrong_mask),
        Err(EncoderError::Shape)
    ));
    let variable = candle_core::Var::zeros((1, 384), DType::F32, &Device::Cpu)
        .expect("synthetic trainable tensor");
    assert!(matches!(
        math::normalize(variable.as_tensor()),
        Err(EncoderError::Precision)
    ));
    assert!(matches!(
        math::normalize(&row(3., 4.).to_dtype(DType::F64).expect("double")),
        Err(EncoderError::Precision)
    ));
}

#[test]
fn rejects_invalid_masks_nonfinite_hidden_states_and_pool_overflow() {
    for mask in [[[0_u32, 0, 0]], [[1_u32, 2, 0]]] {
        assert!(matches!(
            math::masked_mean(
                &hidden_fixture(),
                &Tensor::new(&mask, &Device::Cpu).expect("bad mask")
            ),
            Err(EncoderError::Values)
        ));
    }
    let mask = Tensor::new(&[[1_u32, 1, 0]], &Device::Cpu).expect("mask");
    for value in [f32::NAN, f32::INFINITY, f32::MAX] {
        let mut values = vec![0_f32; 3 * 384];
        values[0] = value;
        values[384] = value;
        let hidden =
            Tensor::from_vec(values, (1, 3, 384), &Device::Cpu).expect("bad hidden fixture");
        assert!(matches!(
            math::masked_mean(&hidden, &mask),
            Err(EncoderError::Values)
        ));
    }
}

fn modules() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!([
        {"idx":0,"name":"0","path":"","type":"sentence_transformers.models.Transformer"},
        {"idx":1,"name":"1","path":"1_Pooling","type":"sentence_transformers.models.Pooling"},
        {"idx":2,"name":"2","path":"2_Normalize","type":"sentence_transformers.models.Normalize"}
    ]))
    .expect("declared module graph")
}

fn pooling() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"word_embedding_dimension":384,"pooling_mode_cls_token":false,
        "pooling_mode_mean_tokens":true,"pooling_mode_max_tokens":false,"pooling_mode_mean_sqrt_len_tokens":false})).expect("declared mean pooling")
}

#[test]
fn accepts_only_the_declared_transformer_mean_pooling_normalize_profile() {
    assert_eq!(profile(&modules(), &pooling()), Ok(()));
}

#[test]
fn rejects_other_module_order_types_paths_and_pooling_modes() {
    for change in 0..3 {
        let mut graph: serde_json::Value = serde_json::from_slice(&modules()).expect("fixture");
        match change {
            0 => graph.as_array_mut().expect("graph").swap(1, 2),
            1 => graph[2]["type"] = serde_json::json!("arbitrary.remote.Class"),
            _ => graph[1]["path"] = serde_json::json!("../other"),
        }
        assert_eq!(
            profile(&serde_json::to_vec(&graph).expect("fixture"), &pooling()),
            Err(EncoderError::Configuration)
        );
    }
    let original: serde_json::Value = serde_json::from_slice(&pooling()).expect("fixture");
    for key in original.as_object().expect("fields").keys() {
        let mut changed = original.clone();
        changed[key] = if key == "word_embedding_dimension" {
            serde_json::json!(768)
        } else {
            serde_json::json!(!changed[key].as_bool().expect("flag"))
        };
        assert_eq!(
            profile(&modules(), &serde_json::to_vec(&changed).expect("fixture")),
            Err(EncoderError::Configuration)
        );
    }
}

#[test]
fn rejects_profile_before_tokenizer_or_checkpoint_construction() {
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
            .expect("synthetic integrity bytes");
    }
    let cache = stage.publish().expect("synthetic publication");
    let path = root.path().join(cache.bundle_id());
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ =
                fs::set_permissions(self.0.join("1_Pooling"), fs::Permissions::from_mode(0o700));
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }
    let _cleanup = Cleanup(path);
    let cache = VerifiedArtifactCache::open(
        parent.open_subdir(cache.bundle_id()).expect("bundle"),
        manifest,
    )
    .expect("synthetic cache");
    // Both later loaders reject this fixture too, but through different variants.
    assert!(matches!(
        CandidateCpuSentenceEncoder::load(&cache),
        Err(EncoderError::Configuration)
    ));
}

fn actual_cache() -> crate::artifact_fs::staging::model::VerifiedArtifactCache {
    use crate::{
        artifact_fs::{staging::model::VerifiedArtifactCache, ArtifactDir},
        model_manifest::PinnedMiniLmManifest,
    };
    let root =
        std::env::var_os("MUNINN_MINILM_ARTIFACT_ROOT").expect("explicit private cache root");
    let parent = ArtifactDir::open_root(std::path::Path::new(&root)).expect("safe existing root");
    let manifest = PinnedMiniLmManifest::load_pinned(std::io::Cursor::new(include_bytes!(
        "../../../../docs/muninn/evidence/artifact-lock.json"
    )))
    .expect("compiled pin");
    VerifiedArtifactCache::open(
        parent
            .open_subdir("ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1")
            .expect("known bundle"),
        manifest,
    )
    .expect("actual artifacts verified")
}

#[test]
#[ignore = "Actual candidate CPU inference only; not reference/C0/C1/numerical acceptance"]
fn actual_candidate_forward_is_finite_and_repeats_without_inference_acceptance() {
    let cache = actual_cache();
    let encoder = CandidateCpuSentenceEncoder::load(&cache).expect("validated candidate model");
    let vectors = encoder.encode(&["", "Hello world!"]);
    assert!(vectors.is_ok());
    let vectors = vectors.expect("actual candidate forward");
    assert_eq!(vectors.len(), 2);
    assert!(vectors
        .iter()
        .all(|row| row.len() == 384 && row.iter().all(|value| value.is_finite())));
    assert_eq!(encoder.artifact_bundle_id(), cache.bundle_id());
    assert!(matches!(
        encoder.encode(&[]),
        Err(EncoderError::Tokenizer(TokenizerError::InputLimit))
    ));
    let repeated = encoder
        .encode(&["", "Hello world!"])
        .expect("same batch repeated");
    assert_eq!(bits(&vectors), bits(&repeated));
    let batch = encoder
        .tokenizer
        .encode(&["", "Hello world!"])
        .expect("exact tokenizer batch");
    let (hidden, mask) = encoder
        .forward(&batch)
        .expect("actual masked hidden states");
    assert_eq!(
        mask.to_vec2::<u32>().expect("actual forwarded mask"),
        batch.attention_mask()
    );
    assert!(!hidden.track_op());
    assert_eq!(hidden.dtype(), DType::F32);
    assert!(hidden.device().is_cpu());
    let original_pooled = math::masked_mean(&hidden, &mask)
        .expect("original raw pooled states")
        .to_vec2::<f32>()
        .expect("pooled values");
    let mut altered = batch.input_ids().to_vec();
    for (ids, mask) in altered.iter_mut().zip(batch.attention_mask()) {
        for (id, keep) in ids.iter_mut().zip(mask) {
            if *keep == 0 {
                *id = 7592;
            }
        }
    }
    let types = input_tensor(batch.token_type_ids()).expect("types");
    let altered_ids = input_tensor(&altered).expect("valid substituted padding IDs");
    let changed = encoder
        .checkpoint
        .model()
        .forward(&altered_ids, &types, Some(&mask))
        .expect("mask-preserving canary");
    let changed_pooled = math::masked_mean(&changed, &mask).expect("masked canary raw mean");
    assert_eq!(
        bits(&original_pooled),
        bits(&changed_pooled.to_vec2::<f32>().expect("raw canary values"))
    );
    let changed = math::normalize(&changed_pooled)
        .expect("normalized canary")
        .to_vec2::<f32>()
        .expect("canary values");
    assert_eq!(bits(&vectors), bits(&changed));
    // Negative control: omit attention masks in BOTH forwards while retaining
    // the original pooling mask. The padding substitution must then matter.
    let original_ids = input_tensor(batch.input_ids()).expect("original IDs");
    let unmasked_original = encoder
        .checkpoint
        .model()
        .forward(&original_ids, &types, None)
        .expect("negative control");
    let unmasked_changed = encoder
        .checkpoint
        .model()
        .forward(&altered_ids, &types, None)
        .expect("negative control substitution");
    let unmasked_original =
        math::normalize(&math::masked_mean(&unmasked_original, &mask).expect("original pooled"))
            .expect("original normalized")
            .to_vec2::<f32>()
            .expect("values");
    let unmasked_changed =
        math::normalize(&math::masked_mean(&unmasked_changed, &mask).expect("changed pooled"))
            .expect("changed normalized")
            .to_vec2::<f32>()
            .expect("values");
    assert_ne!(bits(&unmasked_original), bits(&unmasked_changed));
    let mut variation = Vec::new();
    for (index, text) in ["", "Hello world!"].iter().enumerate() {
        let single = encoder
            .encode(&[*text])
            .expect("single forward measurement");
        variation.push(
            single[0]
                .iter()
                .zip(&vectors[index])
                .map(|(a, b)| (f64::from(*a) - f64::from(*b)).abs())
                .fold(0_f64, f64::max),
        );
    }
    use sha2::{Digest, Sha256};
    let hashes: Vec<_> = vectors
        .iter()
        .map(|row| {
            format!(
                "{:x}",
                Sha256::digest(row.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>())
            )
        })
        .collect();
    let norms: Vec<_> = vectors
        .iter()
        .map(|row| {
            row.iter()
                .map(|v| f64::from(*v).powi(2))
                .sum::<f64>()
                .sqrt()
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({"actual_forward_executed":true,"sentences":2,"dimensions":384,"device":"Cpu","dtype":"F32",
        "gradient_tracking":false,"repeat_bitwise_equal":true,"padding_mask_same_width_invariant":true,"unmasked_control_changed":true,
        "observed_f64_norms":norms,"vector_sha256":hashes,"single_vs_batch_max_abs":variation,"batch_invariance_accepted":false,"C1_accepted":false})
    );
}

fn bits(rows: &[Vec<f32>]) -> Vec<Vec<u32>> {
    rows.iter()
        .map(|row| row.iter().map(|value| value.to_bits()).collect())
        .collect()
}
