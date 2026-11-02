//! Puente TCK experimental; no es la API permanente ni acredita paridad.

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod artifact_fs;
pub mod artifact_route;
pub mod artifacts;
pub mod auth;
pub mod domain;
pub mod embedding_port;
pub mod memory_search;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod minilm_checkpoint;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod minilm_encoder;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod minilm_tokenizer;
pub mod model_manifest;
pub mod storage;

#[cfg(test)]
mod candle_zip_compat;
