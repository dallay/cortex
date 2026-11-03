//! Declaration adapter for C1 tooling, separate from export and execution evidence.
use super::embedding_space::{EmbeddingSpace, ModelFile};
use muninn_bridge_spike::model_manifest::PinnedMiniLmManifest;

/// Ordered verifier receipt digest, without cache access.
pub fn declared_bundle_id(manifest: &PinnedMiniLmManifest) -> Result<String, String> {
    use serde::Serialize;
    use sha2::{Digest, Sha256};
    #[derive(Serialize)]
    struct File<'a> {
        path: &'a str,
        size: u64,
        sha256: &'a str,
    }
    #[derive(Serialize)]
    struct Receipt<'a> {
        schema_version: u32,
        model_id: &'a str,
        revision: &'a str,
        files: Vec<File<'a>>,
    }
    let mut files: Vec<_> = manifest
        .files()
        .iter()
        .map(|f| File {
            path: f.path(),
            size: f.size(),
            sha256: f.sha256(),
        })
        .collect();
    files.sort_unstable_by_key(|f| f.path);
    let bytes = serde_json::to_vec(&Receipt {
        schema_version: 1,
        model_id: "sentence-transformers/all-MiniLM-L6-v2",
        revision: manifest.revision(),
        files,
    })
    .map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn declare(
    manifest: &PinnedMiniLmManifest,
    numeric_contract_version: Option<&str>,
) -> Result<serde_json::Value, String> {
    let contract = numeric_contract_version.ok_or("numeric_contract_version is required")?;
    // C1 uses the same explicit contract grammar as the Python declaration helper.
    let is_alphanumeric = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    if !(1..=64).contains(&contract.len())
        || !contract.bytes().next().is_some_and(is_alphanumeric)
        || !contract
            .bytes()
            .all(|byte| is_alphanumeric(byte) || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("numeric_contract_version must match [a-z0-9][a-z0-9._-]{0,63}".into());
    }
    let mut model_files: Vec<ModelFile> = manifest
        .files()
        .iter()
        .filter(|file| file.path() != "README.md")
        .map(|file| ModelFile {
            path: file.path().into(),
            size: file.size(),
            sha256: file.sha256().into(),
        })
        .collect();
    // Canonical order by path: the serialized declaration must match the Python
    // declaration byte for byte, not only in its derived identifier.
    model_files.sort_by(|a, b| a.path.cmp(&b.path));
    let space = EmbeddingSpace {
        provider_id: "local-cpu".into(),
        model_id: "sentence-transformers/all-MiniLM-L6-v2".into(),
        model_revision: manifest.revision().into(),
        model_files,
        dimensions: 384,
        normalization: "l2".into(),
        similarity_function: "cosine".into(),
        numeric_contract_version: contract.into(),
        embedding_schema_version: 1,
        provisioned_indices: Vec::new(),
    };
    let space_id = space.space_id().map_err(|e| format!("{e:?}"))?;
    space
        .validate_provisioned_indices()
        .map_err(|e| format!("{e:?}"))?;
    let compatibility = serde_json::json!({
        "provider_id": space.provider_id,
        "model_id": space.model_id,
        "model_revision": space.model_revision,
        "model_files": space.model_files.iter().map(|file| serde_json::json!({
            "path": file.path, "size": file.size, "sha256": file.sha256,
        })).collect::<Vec<_>>(),
        "dimensions": space.dimensions,
        "normalization": space.normalization,
        "similarity_function": space.similarity_function,
        "numeric_contract_version": space.numeric_contract_version,
        "embedding_schema_version": space.embedding_schema_version,
    });
    Ok(serde_json::json!({
        "schema_version": 1,
        "artifact_bundle_id": declared_bundle_id(manifest)?,
        "scope": "identity_declaration_not_execution_authentication",
        "space_id": space_id,
        "compatibility": compatibility,
        "provisioned_indices": [],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c1_contract_grammar_matches_shared_cases() {
        let manifest = PinnedMiniLmManifest::load_pinned(
            &include_bytes!("../../../../dev/muninn/fixtures/artifact-lock.json")[..],
        )
        .unwrap();
        let cases: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../fixtures/minilm-c1-contract-cases.json"
        ))
        .unwrap();
        for (category, expected) in [("valid", true), ("invalid", false)] {
            for case in cases[category].as_array().unwrap() {
                let contract = case.as_str().unwrap();
                assert_eq!(
                    declare(&manifest, Some(contract)).is_ok(),
                    expected,
                    "contract: {contract:?}"
                );
            }
        }
    }

    #[test]
    fn python_cross_language_literal_golden() {
        let manifest = PinnedMiniLmManifest::load_pinned(
            &include_bytes!("../../../../dev/muninn/fixtures/artifact-lock.json")[..],
        )
        .unwrap();
        let value = declare(&manifest, Some("test-unapproved-v1")).unwrap();
        assert_eq!(
            value["space_id"],
            "7b39cda61fd78b7fcf492989102d58151950b3cdb1451b89c1a107d3a109ddea"
        );
        assert_eq!(
            value["artifact_bundle_id"],
            "ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1"
        );
    }

    #[test]
    fn declaration_matches_the_shared_python_golden_byte_for_byte() {
        let manifest = PinnedMiniLmManifest::load_pinned(
            &include_bytes!("../../../../dev/muninn/fixtures/artifact-lock.json")[..],
        )
        .unwrap();
        let golden: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../fixtures/minilm-c1-identity-golden.json"
        ))
        .unwrap();
        let declared = declare(&manifest, Some("test-unapproved-v1")).unwrap();
        assert_eq!(declared, golden);
    }

    #[test]
    fn declaration_uses_approved_manifest_without_cache_or_indices() {
        let manifest = PinnedMiniLmManifest::load_pinned(
            &include_bytes!("../../../../dev/muninn/fixtures/artifact-lock.json")[..],
        )
        .unwrap();
        let value = declare(&manifest, Some("test-unapproved-v1")).unwrap();
        let space = &value["compatibility"];
        assert_eq!(
            value["scope"],
            "identity_declaration_not_execution_authentication"
        );
        assert_eq!(space["provider_id"], "local-cpu");
        assert_eq!(space["model_id"], "sentence-transformers/all-MiniLM-L6-v2");
        assert_eq!(space["model_revision"], manifest.revision());
        assert_eq!(space["dimensions"], 384);
        assert_eq!(space["normalization"], "l2");
        assert_eq!(space["similarity_function"], "cosine");
        assert_eq!(space["numeric_contract_version"], "test-unapproved-v1");
        assert_eq!(space["embedding_schema_version"], 1);
        assert_eq!(value["provisioned_indices"], serde_json::json!([]));
        let files = space["model_files"].as_array().unwrap();
        assert_eq!(files.len(), 10);
        for file in manifest.files().iter().filter(|f| f.path() != "README.md") {
            let declared = files.iter().find(|f| f["path"] == file.path()).unwrap();
            assert_eq!(declared["size"], file.size());
            assert_eq!(declared["sha256"], file.sha256());
        }
    }

    #[test]
    fn contract_is_required_and_affects_identity() {
        let manifest = PinnedMiniLmManifest::load_pinned(
            &include_bytes!("../../../../dev/muninn/fixtures/artifact-lock.json")[..],
        )
        .unwrap();
        for contract in [None, Some(""), Some("invalid contract")] {
            assert!(declare(&manifest, contract).is_err());
        }
        let first = declare(&manifest, Some("test-unapproved-v1")).unwrap();
        let second = declare(&manifest, Some("test-unapproved-v2")).unwrap();
        assert_ne!(first["space_id"], second["space_id"]);
    }
}
