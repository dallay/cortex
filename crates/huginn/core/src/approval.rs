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
    for line in preview.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Directory:") {
            let dir = rest.trim();
            if dir.is_empty() {
                return Some("run command in workspace".to_string());
            }
            return Some(format!("run command in workspace ({dir})"));
        }
        if trimmed.starts_with("Trusted local MCP server") {
            return Some(trimmed.to_string());
        }
        if let Some(rest) = trimmed.strip_prefix("Server:") {
            return Some(format!("start MCP server {}", rest.trim()));
        }
        if let Some(rest) = trimmed.strip_prefix("Server/action:") {
            // The first non-empty line carries the MCP server name; the
            // subsequent `Tool:` line carries the remote tool name. Surface
            // both so the user knows which exact tool the approval is for,
            // not just the configured server.
            let server = rest.trim();
            for line in preview.lines().skip(1) {
                if let Some(tool) = line.trim().strip_prefix("Tool:") {
                    let tool = tool.trim();
                    if tool.is_empty() {
                        return Some(format!("call MCP tool {server}"));
                    }
                    return Some(format!("call MCP tool {server}.{tool}"));
                }
            }
            return Some(format!("call MCP tool {server}"));
        }
        if let Some(rest) = trimmed.strip_prefix("MCP tool:") {
            return Some(format!("call MCP tool {}", rest.trim()));
        }
        if let Some(rest) = trimmed.strip_prefix("--- ") {
            let target = rest.split_whitespace().next().unwrap_or("");
            if target.is_empty() {
                return Some("edit file".to_string());
            }
            return Some(format!("edit {target}"));
        }
        if let Some(rest) = trimmed.strip_prefix("+++ ") {
            let target = rest.split_whitespace().next().unwrap_or("");
            if target.is_empty() {
                return Some("create file".to_string());
            }
            return Some(format!("create {target}"));
        }
    }
    None
}
