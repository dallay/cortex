//! Identity tests use the checked-in lock, never the model cache.
use super::{EmbeddingSpace, ModelFile, ProvisionedIndex, SpaceError};

fn pinned_space() -> EmbeddingSpace {
    let lock: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../dev/muninn/fixtures/artifact-lock.json"
    ))
    .unwrap();
    let model = lock["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "sentence-transformers/all-MiniLM-L6-v2")
        .unwrap();
    EmbeddingSpace {
        provider_id: "local-cpu".into(),
        model_id: model["id"].as_str().unwrap().into(),
        model_revision: model["revision"].as_str().unwrap().into(),
        model_files: model["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|file| file["path"] != "README.md")
            .map(|file| ModelFile {
                path: file["path"].as_str().unwrap().into(),
                size: file["size"].as_u64().unwrap(),
                sha256: file["sha256"].as_str().unwrap().into(),
            })
            .collect(),
        dimensions: 384,
        normalization: "l2".into(),
        similarity_function: "cosine".into(),
        // Synthetic contract label, not an approved numerical policy.
        numeric_contract_version: "test-unapproved-v1".into(),
        embedding_schema_version: 1,
        provisioned_indices: Vec::new(),
    }
}

#[test]
fn canonical_identity_is_order_independent_and_explicit() {
    let space = pinned_space();
    let bytes = space.canonical_bytes().unwrap();
    let mut reordered = space.clone();
    reordered.model_files.reverse();
    assert_eq!(bytes, reordered.canonical_bytes().unwrap());
    assert_eq!(space.space_id(), reordered.space_id());
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value[0], "muninn-embedding-space-v1");
    let files = value[4].as_array().unwrap();
    assert_eq!(files.len(), 10);
    assert_eq!(files[0][0], "1_Pooling/config.json");
    assert_eq!(space.space_id().unwrap().len(), 64);
}

#[test]
fn every_compatibility_field_and_every_file_changes_identity() {
    let space = pinned_space();
    let base = space.space_id().unwrap();
    let mut variants = Vec::new();
    let mut changed = space.clone();
    changed.provider_id.push('2');
    variants.push(changed);
    let mut changed = space.clone();
    changed.model_id.push('2');
    variants.push(changed);
    let mut changed = space.clone();
    changed.model_revision = "a".repeat(40);
    variants.push(changed);
    let mut changed = space.clone();
    changed.dimensions += 1;
    variants.push(changed);
    let mut changed = space.clone();
    changed.numeric_contract_version.push('2');
    variants.push(changed);
    let mut changed = space.clone();
    changed.embedding_schema_version += 1;
    variants.push(changed);
    for i in 0..space.model_files.len() {
        let mut changed = space.clone();
        changed.model_files[i].sha256 = "a".repeat(64);
        variants.push(changed);
        let mut changed = space.clone();
        changed.model_files[i].size += 1;
        variants.push(changed);
    }
    for changed in variants {
        assert_ne!(base, changed.space_id().unwrap());
    }
    // Other normalization/similarity values are unsupported, not new valid spaces.
    let mut changed = space.clone();
    changed.normalization = "none".into();
    assert_eq!(changed.space_id(), Err(SpaceError::InvalidNormalization));
    let mut changed = space;
    changed.similarity_function = "dot".into();
    assert_eq!(changed.space_id(), Err(SpaceError::InvalidSimilarity));
}

#[test]
fn files_fail_closed_for_missing_extra_duplicate_paths_sizes_and_digests() {
    let space = pinned_space();
    let mut invalid = space.clone();
    invalid.model_files.pop();
    assert_eq!(invalid.space_id(), Err(SpaceError::FileManifestMismatch));
    let mut invalid = space.clone();
    invalid.model_files.push(space.model_files[0].clone());
    assert_eq!(invalid.space_id(), Err(SpaceError::FileManifestMismatch));
    for i in 0..space.model_files.len() {
        let mut invalid = space.clone();
        invalid.model_files[i].path = "../config.json".into();
        assert_eq!(invalid.space_id(), Err(SpaceError::FileManifestMismatch));
        for size in [0, 128 * 1024 * 1024 + 1, u64::MAX] {
            let mut invalid = space.clone();
            invalid.model_files[i].size = size;
            assert_eq!(invalid.space_id(), Err(SpaceError::FileManifestMismatch));
        }
        for digest in [
            "".into(),
            "a".repeat(63),
            "a".repeat(65),
            "A".repeat(64),
            "g".repeat(64),
        ] {
            let mut invalid = space.clone();
            invalid.model_files[i].sha256 = digest;
            assert_eq!(invalid.space_id(), Err(SpaceError::InvalidDigest));
        }
    }
    let mut invalid = space;
    invalid.model_files[0].path = invalid.model_files[1].path.clone();
    assert_eq!(invalid.space_id(), Err(SpaceError::FileManifestMismatch));
}

#[test]
fn field_invariants_fail_before_hashing() {
    for dimensions in [0, 8193, u32::MAX] {
        let mut space = pinned_space();
        space.dimensions = dimensions;
        assert_eq!(space.space_id(), Err(SpaceError::InvalidDimensions));
    }
    for value in ["".into(), "x".repeat(129), "bad\nname".into()] {
        let mut space = pinned_space();
        space.provider_id = value.clone();
        assert_eq!(space.space_id(), Err(SpaceError::InvalidIdentifier));
        let mut space = pinned_space();
        space.model_id = value.clone();
        assert_eq!(space.space_id(), Err(SpaceError::InvalidIdentifier));
        let mut space = pinned_space();
        space.numeric_contract_version = value;
        assert_eq!(space.space_id(), Err(SpaceError::InvalidContractVersion));
    }
    for revision in [
        "".into(),
        "a".repeat(39),
        "a".repeat(41),
        "A".repeat(40),
        "g".repeat(40),
    ] {
        let mut space = pinned_space();
        space.model_revision = revision;
        assert_eq!(space.space_id(), Err(SpaceError::InvalidRevision));
    }
    let mut space = pinned_space();
    space.embedding_schema_version = 0;
    assert_eq!(space.space_id(), Err(SpaceError::InvalidSchemaVersion));
    for dimensions in [1, 8192] {
        let mut space = pinned_space();
        space.dimensions = dimensions;
        assert!(space.space_id().is_ok());
    }
}

#[test]
fn indices_are_separate_from_identity_and_use_full_ids() {
    let mut space = pinned_space();
    let id = space.space_id().unwrap();
    let bytes = space.canonical_bytes().unwrap();
    for kind in ["vector", "keyword"] {
        let name = space.index_name(kind).unwrap();
        assert_eq!(name, format!("muninn_{kind}_{id}"));
        space.provisioned_indices.push(ProvisionedIndex {
            kind: kind.into(),
            name,
        });
    }
    assert_eq!(space.canonical_bytes().unwrap(), bytes);
    assert_eq!(space.space_id().unwrap(), id);
    assert!(space.validate_provisioned_indices().is_ok());
    space.provisioned_indices.reverse();
    assert!(space.validate_provisioned_indices().is_ok());
    let mut other = pinned_space();
    other.dimensions += 1;
    space.provisioned_indices[0].name = other.index_name("keyword").unwrap();
    assert_eq!(
        space.validate_provisioned_indices(),
        Err(SpaceError::InvalidIndexName)
    );
    assert_eq!(space.space_id().unwrap(), id);
    for kind in ["", "Vector", "bad/name", "bad\nname", &"a".repeat(33)] {
        assert_eq!(space.index_name(kind), Err(SpaceError::InvalidIndexName));
    }
    for name in [
        "".into(),
        format!("muninn_vector_{}", &id[..16]),
        "external_index".into(),
    ] {
        space.provisioned_indices = vec![ProvisionedIndex {
            kind: "vector".into(),
            name,
        }];
        assert_eq!(
            space.validate_provisioned_indices(),
            Err(SpaceError::InvalidIndexName)
        );
    }
    space.provisioned_indices = vec![ProvisionedIndex {
        kind: "bad/name".into(),
        name: id,
    }];
    assert_eq!(
        space.validate_provisioned_indices(),
        Err(SpaceError::InvalidIndexName)
    );
    let index = ProvisionedIndex {
        kind: "vector".into(),
        name: space.index_name("vector").unwrap(),
    };
    space.provisioned_indices = vec![index.clone(), index];
    assert_eq!(
        space.validate_provisioned_indices(),
        Err(SpaceError::InvalidIndexName)
    );
}

#[test]
fn canonical_identity_matches_independent_synthetic_golden() {
    // Ten synthetic file records: not model artifacts or numerical-policy evidence.
    // Independent oracle: Python stdlib json.dumps(value, ensure_ascii=False,
    // separators=(",", ":")).encode("utf-8"), then hashlib.sha256(bytes).hexdigest().
    // value is the versioned tuple below, with file tuples sorted by path;
    // decimal integers, compact JSON, and no trailing newline. No Rust output used.
    let model_files = [
        (
            "vocab.txt",
            10,
            "9999999999999999999999999999999999999999999999999999999999999999",
        ),
        (
            "tokenizer_config.json",
            9,
            "8888888888888888888888888888888888888888888888888888888888888888",
        ),
        (
            "tokenizer.json",
            8,
            "7777777777777777777777777777777777777777777777777777777777777777",
        ),
        (
            "special_tokens_map.json",
            7,
            "6666666666666666666666666666666666666666666666666666666666666666",
        ),
        (
            "sentence_bert_config.json",
            6,
            "5555555555555555555555555555555555555555555555555555555555555555",
        ),
        (
            "modules.json",
            5,
            "4444444444444444444444444444444444444444444444444444444444444444",
        ),
        (
            "model.safetensors",
            4,
            "3333333333333333333333333333333333333333333333333333333333333333",
        ),
        (
            "config_sentence_transformers.json",
            3,
            "2222222222222222222222222222222222222222222222222222222222222222",
        ),
        (
            "config.json",
            2,
            "1111111111111111111111111111111111111111111111111111111111111111",
        ),
        (
            "1_Pooling/config.json",
            1,
            "0000000000000000000000000000000000000000000000000000000000000000",
        ),
    ]
    .into_iter()
    .map(|(path, size, sha256)| ModelFile {
        path: path.into(),
        size,
        sha256: sha256.into(),
    })
    .collect();
    let space = EmbeddingSpace {
        provider_id: "synthetic-provider".into(),
        model_id: "synthetic-model".into(),
        model_revision: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        model_files,
        dimensions: 384,
        normalization: "l2".into(),
        similarity_function: "cosine".into(),
        numeric_contract_version: "synthetic-unapproved-v1".into(),
        embedding_schema_version: 1,
        provisioned_indices: Vec::new(),
    };
    let expected_bytes = br#"["muninn-embedding-space-v1","synthetic-provider","synthetic-model","aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",[["1_Pooling/config.json",1,"0000000000000000000000000000000000000000000000000000000000000000"],["config.json",2,"1111111111111111111111111111111111111111111111111111111111111111"],["config_sentence_transformers.json",3,"2222222222222222222222222222222222222222222222222222222222222222"],["model.safetensors",4,"3333333333333333333333333333333333333333333333333333333333333333"],["modules.json",5,"4444444444444444444444444444444444444444444444444444444444444444"],["sentence_bert_config.json",6,"5555555555555555555555555555555555555555555555555555555555555555"],["special_tokens_map.json",7,"6666666666666666666666666666666666666666666666666666666666666666"],["tokenizer.json",8,"7777777777777777777777777777777777777777777777777777777777777777"],["tokenizer_config.json",9,"8888888888888888888888888888888888888888888888888888888888888888"],["vocab.txt",10,"9999999999999999999999999999999999999999999999999999999999999999"]],384,"l2","cosine","synthetic-unapproved-v1",1]"#;
    assert_eq!(space.canonical_bytes().unwrap(), expected_bytes);
    assert_eq!(
        space.space_id().unwrap(),
        "e35d1bbf64ad8c0356298387b3ac9cbc9b9d1d36d0385df8587820674aec4538"
    );
}
