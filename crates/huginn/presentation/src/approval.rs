//! Shared approval presentation model: header numbering and effect summaries.
//!
//! Both interactive adapters (the line adapter in `apps/huginn` and the
//! Ratatui presentation plugin) derive the human-visible approval header from
//! these functions so numbering and `Effect:` semantics cannot drift between
//! them. These helpers carry **no authority**: only an `ApprovalPolicy`
//! implementation grants or denies an effect.
//!
//! Callers pass sanitized text. Sanitizing is idempotent, so sanitizing
//! before calling keeps byte-identical output for already-clean input.

/// `Approval for <action> (request #N this turn):`.
///
/// The counter resets at the start of each turn so every turn numbers its
/// first approval `#1`. The number never claims that a later request differs
/// from an earlier one; it only positions the request within the turn.
pub fn approval_header(action: &str, number: u64) -> String {
    format!("Approval for {action} (request #{number} this turn):")
}

/// Derive a single short `Effect:` line from a preview.
///
/// Uses the first non-empty line when it starts with a known prefix.
/// Conservative by design: returning `None` is always safe (the header
/// omits the `Effect:` line).
pub fn effect_line(preview: &str) -> Option<String> {
    for (index, line) in preview.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(effect) = directory_effect(trimmed)
            .or_else(|| trusted_server_effect(trimmed))
            .or_else(|| server_effect(trimmed))
            .or_else(|| server_action_effect(preview, index, trimmed))
            .or_else(|| mcp_tool_effect(trimmed))
            .or_else(|| file_effect(trimmed, "--- ", "edit file", "edit"))
            .or_else(|| file_effect(trimmed, "+++ ", "create file", "create"))
        {
            return Some(effect);
        }
    }
    None
}

fn directory_effect(line: &str) -> Option<String> {
    let rest = line.strip_prefix("Directory:")?;
    let directory = rest.trim();
    Some(if directory.is_empty() {
        "run command in workspace".to_string()
    } else {
        format!("run command in workspace ({directory})")
    })
}

fn trusted_server_effect(line: &str) -> Option<String> {
    line.starts_with("Trusted local MCP server")
        .then(|| line.to_string())
}

fn server_effect(line: &str) -> Option<String> {
    let rest = line.strip_prefix("Server:")?;
    Some(format!("start MCP server {}", rest.trim()))
}

fn server_action_effect(preview: &str, index: usize, line: &str) -> Option<String> {
    let server = line.strip_prefix("Server/action:")?.trim();
    let tool = preview
        .lines()
        .skip(index + 1)
        .find_map(|line| line.trim().strip_prefix("Tool:"))
        .map(str::trim);
    Some(match tool {
        Some("") | None => format!("call MCP tool {server}"),
        Some(tool) => format!("call MCP tool {server}.{tool}"),
    })
}

fn mcp_tool_effect(line: &str) -> Option<String> {
    let rest = line.strip_prefix("MCP tool:")?;
    Some(format!("call MCP tool {}", rest.trim()))
}

fn file_effect(line: &str, prefix: &str, empty: &str, action: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?;
    let target = rest.split_whitespace().next().unwrap_or("");
    Some(if target.is_empty() {
        empty.to_string()
    } else {
        format!("{action} {target}")
    })
}
