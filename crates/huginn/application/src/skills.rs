//! Read-only skill catalog contracts. Metadata never grants host authority.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceScope {
    Project,
    Global,
}
impl SourceScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}

/// Catalog text is never an authorization or model trust decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillTrust {
    Untrusted,
}
impl SkillTrust {
    pub const fn as_str(self) -> &'static str {
        "untrusted"
    }
}

#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub source_scope: SourceScope,
    pub logical_path: PathBuf,
    pub canonical_path: PathBuf,
    pub content_fingerprint: String,
    pub selected: bool,
    pub trust: SkillTrust,
}

#[derive(Debug, Clone)]
pub struct SkillDiagnostic {
    pub code: &'static str,
    pub scope: SourceScope,
    pub path: PathBuf,
    pub message: String,
    pub severity: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct SkillCatalog {
    pub skills: Vec<SkillEntry>,
    pub diagnostics: Vec<SkillDiagnostic>,
    pub fingerprint: String,
}
impl SkillCatalog {
    /// Retain both sources for inspection; select only the first valid exact name.
    pub fn select_sources(&mut self) {
        self.skills.sort_by(|a, b| {
            (&a.name, a.source_scope, &a.logical_path).cmp(&(
                &b.name,
                b.source_scope,
                &b.logical_path,
            ))
        });
        let mut previous: Option<&str> = None;
        for entry in &mut self.skills {
            entry.selected = previous != Some(entry.name.as_str());
            previous = Some(&entry.name);
        }
        self.diagnostics
            .sort_by(|a, b| (a.scope, &a.path, a.code).cmp(&(b.scope, &b.path, b.code)));
    }
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "fingerprint": self.fingerprint,
            "skills": self.skills.iter().map(|s| serde_json::json!({
                "name": s.name, "description": s.description,
                "source_scope": s.source_scope.as_str(), "logical_path": s.logical_path.to_string_lossy(),
                "canonical_path": s.canonical_path.to_string_lossy(), "content_fingerprint": s.content_fingerprint,
                "trust": s.trust.as_str(), "selected": s.selected,
                "status": if s.selected { "selected" } else { "overridden" }
            })).collect::<Vec<_>>(),
            "diagnostics": self.diagnostics.iter().map(|d| serde_json::json!({
                "code": d.code, "scope": d.scope.as_str(), "path": d.path.to_string_lossy(),
                "message": d.message, "severity": d.severity
            })).collect::<Vec<_>>()
        })
    }
}

/// Adapter supplies fresh snapshots. Explicit refresh must re-read source bytes.
pub trait SkillCatalogQuery {
    fn refresh(&self, workspace: &Path, home: Option<&Path>) -> std::io::Result<SkillCatalog>;
}
