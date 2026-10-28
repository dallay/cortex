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
pub mod sessions;
pub mod skills;
pub mod tools;
#[cfg(feature = "experimental-wasm")]
pub mod wasm;
