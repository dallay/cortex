//! C1 identity declaration only. Does not load a model or attest an execution.
#[path = "../src/embedding_space.rs"]
mod embedding_space;
#[path = "../src/minilm_c1_identity.rs"]
mod minilm_c1_identity;

use muninn_bridge_spike::model_manifest::PinnedMiniLmManifest;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 1 {
        return Err("usage: declare_minilm_c1_identity <explicit-numeric-contract-version>".into());
    }
    let manifest = PinnedMiniLmManifest::load_pinned(
        &include_bytes!("../../../../docs/muninn/evidence/artifact-lock.json")[..],
    )
    .map_err(|e| format!("{e:?}"))?;
    let declaration = minilm_c1_identity::declare(&manifest, Some(&args[0]))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&declaration).map_err(|e| e.to_string())?
    );
    Ok(())
}
