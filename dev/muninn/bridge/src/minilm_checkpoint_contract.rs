use super::CheckpointError;
use candle_transformers::models::bert::{Config, HiddenAct, PositionEmbeddingType};
use safetensors::tensor::TensorView;
use safetensors::Dtype;
use serde::Deserialize;
use std::collections::BTreeMap;

pub(super) fn inventory<'a>(
    names: impl Iterator<Item = &'a str>,
) -> Result<BTreeMap<String, (Dtype, Vec<usize>)>, CheckpointError> {
    let specs = specifications();
    let mut seen = std::collections::BTreeSet::new();
    for name in names {
        if !specs.contains_key(name) || !seen.insert(name) {
            return Err(CheckpointError::TensorInventory);
        }
    }
    if seen.len() != specs.len() {
        return Err(CheckpointError::TensorInventory);
    }
    Ok(specs)
}

pub(super) fn tensors(bytes: &[u8]) -> Result<safetensors::SafeTensors<'_>, CheckpointError> {
    if u64::try_from(bytes.len()).map_err(|_| CheckpointError::TensorFormat)?
        > crate::artifacts::MAX_ARTIFACT_BYTES
    {
        return Err(CheckpointError::TensorFormat);
    }
    let prefix: [u8; 8] = bytes
        .get(..8)
        .ok_or(CheckpointError::TensorFormat)?
        .try_into()
        .map_err(|_| CheckpointError::TensorFormat)?;
    let length =
        usize::try_from(u64::from_le_bytes(prefix)).map_err(|_| CheckpointError::TensorFormat)?;
    if length > 1024 * 1024 || length.checked_add(8).is_none_or(|end| end > bytes.len()) {
        return Err(CheckpointError::TensorFormat);
    }
    safetensors::SafeTensors::deserialize(bytes).map_err(|_| CheckpointError::TensorFormat)
}

pub(super) fn tensor(
    expected: &(Dtype, Vec<usize>),
    view: &TensorView<'_>,
) -> Result<(), CheckpointError> {
    if view.dtype() != expected.0 {
        return Err(CheckpointError::TensorDtype);
    }
    if view.shape() != expected.1 {
        return Err(CheckpointError::TensorShape);
    }
    match expected.0 {
        Dtype::F32 => finite_f32(view.data()),
        Dtype::I64 => {
            if view.data().len() != 512 * 8 {
                return Err(CheckpointError::TensorValues);
            }
            for (index, bytes) in view.data().as_chunks::<8>().0.iter().enumerate() {
                if i64::from_le_bytes(*bytes)
                    != i64::try_from(index).map_err(|_| CheckpointError::TensorValues)?
                {
                    return Err(CheckpointError::TensorValues);
                }
            }
            Ok(())
        }
        _ => Err(CheckpointError::TensorDtype),
    }
}

pub(super) fn specifications() -> BTreeMap<String, (Dtype, Vec<usize>)> {
    let mut specs = BTreeMap::new();
    // Dimensions come only from this validated fixed MiniLM profile, never
    // from unchecked checkpoint/configuration input.
    for (name, shape) in [
        ("word_embeddings.weight", vec![30522, 384]),
        ("position_embeddings.weight", vec![512, 384]),
        ("token_type_embeddings.weight", vec![2, 384]),
        ("LayerNorm.weight", vec![384]),
        ("LayerNorm.bias", vec![384]),
    ] {
        specs.insert(format!("embeddings.{name}"), (Dtype::F32, shape));
    }
    specs.insert(
        "embeddings.position_ids".to_owned(),
        (Dtype::I64, vec![1, 512]),
    );
    for layer in 0..6 {
        let prefix = format!("encoder.layer.{layer}");
        for (part, output, input) in [
            ("attention.self.query", 384, 384),
            ("attention.self.key", 384, 384),
            ("attention.self.value", 384, 384),
            ("attention.output.dense", 384, 384),
            ("intermediate.dense", 1536, 384),
            ("output.dense", 384, 1536),
        ] {
            specs.insert(
                format!("{prefix}.{part}.weight"),
                (Dtype::F32, vec![output, input]),
            );
            specs.insert(format!("{prefix}.{part}.bias"), (Dtype::F32, vec![output]));
        }
        for part in ["attention.output.LayerNorm", "output.LayerNorm"] {
            for suffix in ["weight", "bias"] {
                specs.insert(format!("{prefix}.{part}.{suffix}"), (Dtype::F32, vec![384]));
            }
        }
    }
    specs.insert(
        "pooler.dense.weight".to_owned(),
        (Dtype::F32, vec![384, 384]),
    );
    specs.insert("pooler.dense.bias".to_owned(), (Dtype::F32, vec![384]));
    specs
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    #[serde(rename = "_name_or_path")]
    name: String,
    architectures: Vec<String>,
    attention_probs_dropout_prob: f64,
    gradient_checkpointing: bool,
    hidden_act: String,
    hidden_dropout_prob: f64,
    hidden_size: usize,
    initializer_range: f64,
    intermediate_size: usize,
    layer_norm_eps: f64,
    max_position_embeddings: usize,
    model_type: String,
    num_attention_heads: usize,
    num_hidden_layers: usize,
    pad_token_id: usize,
    position_embedding_type: String,
    transformers_version: String,
    type_vocab_size: usize,
    use_cache: bool,
    vocab_size: usize,
}

pub(super) fn finite_f32(bytes: &[u8]) -> Result<(), CheckpointError> {
    let (chunks, remainder) = bytes.as_chunks::<4>();
    if chunks
        .iter()
        .any(|chunk| !f32::from_le_bytes(*chunk).is_finite())
        || !remainder.is_empty()
    {
        return Err(CheckpointError::TensorValues);
    }
    Ok(())
}

pub(super) fn config(bytes: &[u8]) -> Result<Config, CheckpointError> {
    if bytes.len() > 16 * 1024 {
        return Err(CheckpointError::Configuration);
    }
    let p: Profile = serde_json::from_slice(bytes).map_err(|_| CheckpointError::Configuration)?;
    if p.name != "nreimers/MiniLM-L6-H384-uncased"
        || p.architectures != ["BertModel"]
        || p.attention_probs_dropout_prob != 0.1
        || p.gradient_checkpointing
        || p.hidden_act != "gelu"
        || p.hidden_dropout_prob != 0.1
        || p.hidden_size != 384
        || p.initializer_range != 0.02
        || p.intermediate_size != 1536
        || p.layer_norm_eps != 1e-12
        || p.max_position_embeddings != 512
        || p.model_type != "bert"
        || p.num_attention_heads != 12
        || p.num_hidden_layers != 6
        || p.pad_token_id != 0
        || p.position_embedding_type != "absolute"
        || p.transformers_version != "4.8.2"
        || p.type_vocab_size != 2
        || !p.use_cache
        || p.vocab_size != 30522
    {
        return Err(CheckpointError::Configuration);
    }
    Ok(Config {
        vocab_size: 30522,
        hidden_size: 384,
        num_hidden_layers: 6,
        num_attention_heads: 12,
        intermediate_size: 1536,
        hidden_act: HiddenAct::Gelu,
        hidden_dropout_prob: 0.1,
        max_position_embeddings: 512,
        type_vocab_size: 2,
        initializer_range: 0.02,
        layer_norm_eps: 1e-12,
        pad_token_id: 0,
        position_embedding_type: PositionEmbeddingType::Absolute,
        use_cache: true,
        classifier_dropout: None,
        model_type: Some("bert".to_owned()),
    })
}
