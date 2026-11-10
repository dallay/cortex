//! Concrete capabilities for Huginn, the Cortex coding agent, independent of the CLI.
pub mod composition;
pub mod connection_config;
pub mod connections;
pub mod guardrails;
pub mod instructions;
pub mod loop_engine;
pub mod mcp;
pub mod model;
pub mod prompts;
pub mod providers;
#[cfg(any(test, target_os = "macos"))]
#[allow(dead_code)] // Runtime consumers are added in the assigned launcher slice.
mod seatbelt;
pub mod sessions;
pub mod skills;
pub mod tools;
#[cfg(feature = "experimental-wasm")]
pub mod wasm;
