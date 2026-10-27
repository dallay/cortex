//! Protecciones restrictivas para herramientas nativas; no constituyen un sandbox.
mod paths;
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

#[derive(Default)]
pub struct Guardrails {
    block_paths: Vec<PathBuf>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Configuration {
    block_paths: Vec<String>,
}
impl Guardrails {
    /// Construye restricciones aditivas desde texto; no lee un archivo de proyecto.
    /// Solo acepta `block_paths`; los campos desconocidos producen un error.
    pub fn from_toml(text: &str) -> std::result::Result<Self, &'static str> {
        if text.len() > 65_536 {
            return Err("guardrails: configuration exceeds 64 KiB");
        }
        let config: Configuration =
            toml::from_str(text).map_err(|_| "guardrails: invalid configuration")?;
        if config.block_paths.len() > 128 {
            return Err("guardrails: too many project rules");
        }
        if config.block_paths.iter().any(|s| {
            s.is_empty()
                || s.len() > 1024
                || s.chars().any(char::is_control)
                || Path::new(s)
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
        }) {
            return Err("guardrails: blocked paths must be relative without traversal");
        }
        Ok(Self {
            block_paths: config.block_paths.into_iter().map(Into::into).collect(),
        })
    }
    /// Comprueba nombres sensibles y prefijos del proyecto, sin conceder acceso.
    /// Resuelve aliases y el ancestro existente de destinos nuevos.
    /// Los enlaces colgantes fallan; no ofrece confinamiento atómico frente a carreras.
    pub fn check_path(
        &self,
        workspace: &Path,
        requested: &Path,
    ) -> std::result::Result<(), &'static str> {
        self.check_lexical_path(requested.strip_prefix(workspace).unwrap_or(requested))?;
        let candidate = workspace.join(requested);
        let resolved = paths::resolve_nearest(&candidate)?;
        let root = workspace
            .canonicalize()
            .map_err(|_| "guardrails: workspace unavailable")?;
        self.check_lexical_path(resolved.strip_prefix(&root).unwrap_or(&resolved))
    }
    fn check_lexical_path(&self, path: &Path) -> std::result::Result<(), &'static str> {
        if paths::sensitive(path)
            || self.block_paths.iter().any(|blocked| {
                paths::starts_with_ascii_case_insensitive(&paths::normalized(path), blocked)
            })
        {
            return Err("guardrails: sensitive path blocked");
        }
        Ok(())
    }
}
