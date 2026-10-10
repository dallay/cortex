use async_trait::async_trait;
use huginn_core::{
    AgentError, ApprovalRequest, PreparedAction, Result, Tool, ToolCall, ToolContext,
    ToolDefinition, ToolRegistry,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

pub const MAX_FILE_BYTES: u64 = 1_048_576;
pub const MAX_OUTPUT_BYTES: usize = 65_536;
#[derive(Default)]
pub struct Registry {
    tools: RwLock<BTreeMap<String, Arc<dyn Tool>>>,
}
impl Registry {
    pub fn native(timeout_secs: u64) -> Result<Self> {
        let registry = Self::default();
        for kind in [
            Kind::List,
            Kind::Read,
            Kind::Search,
            Kind::Write,
            Kind::Edit,
            Kind::Shell,
        ] {
            registry.insert(Arc::new(NativeTool { kind, timeout_secs }))?;
        }
        Ok(registry)
    }
    pub fn insert(&self, tool: Arc<dyn Tool>) -> Result<()> {
        let name = tool.definition().name;
        let mut tools = self
            .tools
            .write()
            .map_err(|_| AgentError::Tool("registry unavailable".into()))?;
        if tools.contains_key(&name) {
            return Err(AgentError::Tool(format!("duplicate tool {name}")));
        }
        tools.insert(name, tool);
        drop(tools);
        Ok(())
    }
}
impl ToolRegistry for Registry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .read()
            .map(|tools| tools.values().map(|t| t.definition()).collect())
            .unwrap_or_default()
    }
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.read().ok()?.get(name).cloned()
    }
}

pub fn resolve_path(root: &Path, path: &str, allow_new: bool) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    let candidate = root.join(path);
    let resolved = match candidate.canonicalize() {
        Ok(value) => value,
        Err(error) if allow_new && error.kind() == std::io::ErrorKind::NotFound => {
            let parent = candidate
                .parent()
                .ok_or_else(|| AgentError::Tool("invalid path".into()))?
                .canonicalize()?;
            let name = candidate
                .file_name()
                .ok_or_else(|| AgentError::Tool("invalid filename".into()))?;
            parent.join(name)
        }
        Err(error) => return Err(error.into()),
    };
    if !resolved.starts_with(&root) {
        return Err(AgentError::Tool(
            "path escapes the authorized workspace".into(),
        ));
    }
    // A dangling symlink must never be mistaken for a new regular file.
    if allow_new
        && candidate
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        && !candidate.exists()
    {
        return Err(AgentError::Tool(
            "dangling symlink is not a writable file".into(),
        ));
    }
    Ok(resolved)
}
fn argument<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .ok_or_else(|| AgentError::Tool(format!("{name} must be a string")))
}
pub fn bounded_text(mut text: String) -> String {
    if text.len() > MAX_OUTPUT_BYTES {
        let mut end = MAX_OUTPUT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[output truncated]");
    }
    text
}
fn skipped(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".pnpm-store" | ".cache"
    )
}
pub fn workspace_files(root: &Path, path: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![path.to_path_buf()];
    let mut files = vec![];
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        visited += 1;
        if visited > 20_000 {
            return Err(AgentError::Tool(
                "search exceeds 20000 entries; select a narrower path".into(),
            ));
        }
        if path.is_file() {
            files.push(path);
            continue;
        }
        let mut entries = std::fs::read_dir(&path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries.into_iter().rev() {
            let kind = entry.file_type()?;
            if skipped(&entry.file_name().to_string_lossy()) || (!kind.is_file() && !kind.is_dir())
            {
                continue;
            }
            let candidate = entry.path();
            if candidate.starts_with(root) {
                pending.push(candidate);
            }
        }
    }
    files.sort();
    Ok(files)
}
#[derive(Clone, Copy)]
enum Kind {
    List,
    Read,
    Search,
    Write,
    Edit,
    Shell,
}
struct NativeTool {
    kind: Kind,
    timeout_secs: u64,
}
#[async_trait]
impl Tool for NativeTool {
    fn definition(&self) -> ToolDefinition {
        let (name,description,properties,required)=match self.kind {
            Kind::List=>("list_files","List up to 100 regular files recursively within a workspace path; narrow the path for larger trees (skips build/dependency directories).",json!({"path":{"type":"string"}}),vec![]),
            Kind::Read=>("read_file","Read a UTF-8 file; use start_line/end_line for a bounded range.",json!({"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1}}),vec!["path"]),
            Kind::Search=>("search_files","Search literal text in UTF-8 files under a workspace path.",json!({"path":{"type":"string"},"query":{"type":"string","minLength":1}}),vec!["query"]),
            Kind::Write=>("write_file","Create or replace a UTF-8 file. Shows a diff and requires approval. Parent directory must exist.",json!({"path":{"type":"string"},"content":{"type":"string"}}),vec!["path","content"]),
            Kind::Edit=>("edit_file","Replace exactly one occurrence of old_text with new_text. Shows a diff and requires approval.",json!({"path":{"type":"string"},"old_text":{"type":"string","minLength":1},"new_text":{"type":"string"}}),vec!["path","old_text","new_text"]),
            Kind::Shell=>("shell","Run a POSIX shell command in the workspace, with explicit approval and bounded time/output. This is not sandboxed.",json!({"command":{"type":"string","minLength":1}}),vec!["command"]),
        };
        ToolDefinition {
            name: name.into(),
            description: description.into(),
            input_schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
        }
    }
    async fn prepare(&self, call: &ToolCall, ctx: &ToolContext) -> Result<PreparedAction> {
        if !call.arguments.is_object() {
            return Err(AgentError::Tool("arguments must be an object".into()));
        }
        if matches!(self.kind, Kind::Shell) {
            let command = argument(&call.arguments, "command")?;
            if command.trim().is_empty() || command.len() > 16_384 {
                return Err(AgentError::Tool("invalid command length".into()));
            }
            return Ok(PreparedAction {
                approval: Some(ApprovalRequest {
                    id: call.id.clone(),
                    action: "native.shell".into(),
                    preview: format!("Directory: {}\nCommand: {command}", ctx.workspace.display()),
                }),
                payload: call.arguments.clone(),
            });
        }
        let path = call.arguments["path"].as_str().unwrap_or(".");
        if matches!(self.kind, Kind::Read | Kind::Write | Kind::Edit) {
            argument(&call.arguments, "path")?;
        }
        let resolved = resolve_path(&ctx.workspace, path, matches!(self.kind, Kind::Write))?;
        let mut payload = call.arguments.clone();
        payload["resolved"] = json!(resolved);
        if matches!(self.kind, Kind::Write | Kind::Edit) {
            let original = match tokio::fs::metadata(&resolved).await {
                Ok(metadata) => {
                    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
                        return Err(AgentError::Tool(
                            "edit target must be a regular file under 1 MiB".into(),
                        ));
                    }
                    Some(tokio::fs::read_to_string(&resolved).await?)
                }
                Err(e)
                    if matches!(self.kind, Kind::Write)
                        && e.kind() == std::io::ErrorKind::NotFound =>
                {
                    None
                }
                Err(e) => return Err(e.into()),
            };
            let replacement = if matches!(self.kind, Kind::Write) {
                argument(&payload, "content")?.to_string()
            } else {
                let old = argument(&payload, "old_text")?;
                let new = argument(&payload, "new_text")?;
                let text = original
                    .as_deref()
                    .ok_or_else(|| AgentError::Tool("edit target missing".into()))?;
                if old.is_empty() || text.matches(old).count() != 1 {
                    return Err(AgentError::Tool("old_text must match exactly once".into()));
                }
                text.replacen(old, new, 1)
            };
            if replacement.len() > MAX_FILE_BYTES as usize {
                return Err(AgentError::Tool("new content exceeds 1 MiB".into()));
            }
            let diff =
                similar::TextDiff::from_lines(original.as_deref().unwrap_or(""), &replacement)
                    .unified_diff()
                    .header(path, path)
                    .to_string();
            // Approval must cover the complete diff, not a silently truncated preview.
            if diff.len() > MAX_OUTPUT_BYTES {
                return Err(AgentError::Tool(
                    "diff too large; split the change into smaller edits".into(),
                ));
            }
            payload["original"] = json!(original);
            payload["replacement"] = json!(replacement);
            return Ok(PreparedAction {
                approval: Some(ApprovalRequest {
                    id: call.id.clone(),
                    action: format!("native.{}", self.definition().name),
                    preview: diff,
                }),
                payload,
            });
        }
        Ok(PreparedAction {
            approval: None,
            payload,
        })
    }
    async fn execute(&self, action: PreparedAction, ctx: &ToolContext) -> Result<String> {
        if ctx.cancellation.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let args = action.payload;
        if matches!(self.kind, Kind::Shell) {
            return run_shell(argument(&args, "command")?, ctx, self.timeout_secs).await;
        }
        let raw = args["path"].as_str().unwrap_or(".");
        let path = resolve_path(&ctx.workspace, raw, matches!(self.kind, Kind::Write))?;
        if json!(path) != args["resolved"] {
            return Err(AgentError::Tool(
                "path changed after preparation; request a new approval".into(),
            ));
        }
        match self.kind {
            Kind::Write | Kind::Edit => execute_mutation(&args, &path).await,
            Kind::Read => execute_read(&args, &path).await,
            Kind::List => execute_list(&path, ctx).await,
            Kind::Search => execute_search(&args, &path, ctx).await,
            Kind::Shell => unreachable!(),
        }
    }
}

async fn execute_mutation(args: &Value, path: &Path) -> Result<String> {
    let existing_metadata = tokio::fs::metadata(path).await.ok();
    if let Some(metadata) = &existing_metadata {
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err(AgentError::Tool(
                "edit target changed; prepare a new diff".into(),
            ));
        }
    }
    let current = match tokio::fs::read_to_string(path).await {
        Ok(value) => Some(value),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if json!(current) != args["original"] {
        return Err(AgentError::Tool(
            "file changed after approval; prepare a new diff".into(),
        ));
    }
    let temp = path.with_file_name(format!(".agent-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Existing files keep their permissions. New files use a normal
        // creation mode so the kernel applies the process umask.
        let mode = existing_metadata
            .as_ref()
            .map(|metadata| metadata.permissions().mode())
            .unwrap_or(0o666);
        options.mode(mode);
    }
    let mut file = options.open(&temp).await?;
    use tokio::io::AsyncWriteExt;
    let result = async {
        file.write_all(argument(args, "replacement")?.as_bytes())
            .await?;
        file.sync_all().await?;
        if let Some(metadata) = &existing_metadata {
            tokio::fs::set_permissions(&temp, metadata.permissions()).await?;
        }
        tokio::fs::rename(&temp, path).await?;
        Ok::<_, AgentError>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    result?;
    Ok(format!("Updated {}", path.display()))
}

async fn execute_read(args: &Value, path: &Path) -> Result<String> {
    let metadata = tokio::fs::metadata(path).await?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(AgentError::Tool(
            "read target must be a regular file under 1 MiB".into(),
        ));
    }
    let text = tokio::fs::read_to_string(path).await?;
    let start = args["start_line"].as_u64().unwrap_or(1);
    let end = args["end_line"]
        .as_u64()
        .unwrap_or_else(|| start.saturating_add(199));
    if start == 0 || end < start {
        return Err(AgentError::Tool("invalid line range".into()));
    }
    Ok(bounded_text(
        text.lines()
            .enumerate()
            .filter(|(i, _)| (*i as u64) >= start - 1 && (*i as u64) < end)
            .map(|(i, line)| format!("{}: {line}", i + 1))
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

async fn workspace_file_list(path: &Path, ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let root = ctx.workspace.clone();
    let input = path.to_path_buf();
    tokio::task::spawn_blocking(move || workspace_files(&root, &input))
        .await
        .map_err(|_| AgentError::Tool("file traversal failed".into()))?
}

async fn execute_list(path: &Path, ctx: &ToolContext) -> Result<String> {
    let files = workspace_file_list(path, ctx).await?;
    let mut listing = files
        .iter()
        .take(100)
        .map(|p| {
            p.strip_prefix(&ctx.workspace)
                .unwrap_or(p)
                .display()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    if files.len() > 100 {
        listing.push_str("\n[100-file limit; select a narrower path]");
    }
    Ok(bounded_text(listing))
}

async fn execute_search(args: &Value, path: &Path, ctx: &ToolContext) -> Result<String> {
    let files = workspace_file_list(path, ctx).await?;
    let query = argument(args, "query")?;
    if query.is_empty() {
        return Err(AgentError::Tool("query must not be empty".into()));
    }
    let mut matches = vec![];
    for file in files {
        if ctx.cancellation.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        if tokio::fs::metadata(&file).await?.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(text) = tokio::fs::read_to_string(&file).await else {
            continue;
        };
        for (index, line) in text
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(query))
        {
            matches.push(format!(
                "{}:{}: {line}",
                file.strip_prefix(&ctx.workspace).unwrap_or(&file).display(),
                index + 1
            ));
            if matches.len() >= 100 {
                return Ok(bounded_text(format!(
                    "{}\n[100-match limit]",
                    matches.join("\n")
                )));
            }
        }
    }
    Ok(bounded_text(matches.join("\n")))
}

async fn run_shell(command: &str, ctx: &ToolContext, timeout_secs: u64) -> Result<String> {
    use process_wrap::tokio::{CommandWrap, KillOnDrop, ProcessGroup};
    use tokio::{io::AsyncReadExt, process::Command};
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(&ctx.workspace)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut wrapped = CommandWrap::from(cmd);
    wrapped.wrap(ProcessGroup::leader()).wrap(KillOnDrop);
    struct ChildGuard(Box<dyn process_wrap::tokio::ChildWrapper>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.start_kill();
        }
    }
    let mut child = ChildGuard(wrapped.spawn()?);
    let stdout = child
        .0
        .stdout()
        .take()
        .ok_or_else(|| AgentError::Tool("missing stdout".into()))?;
    let stderr = child
        .0
        .stderr()
        .take()
        .ok_or_else(|| AgentError::Tool("missing stderr".into()))?;
    // Each stream gets its own budget so stderr stays visible when stdout is full.
    // Reserve space for the "Exit/stdout/stderr" header; `bounded_text` remains
    // the final safety limit for the combined output.
    // Keep draining after the cap so a verbose child cannot block on a full pipe.
    async fn drain(
        mut reader: impl tokio::io::AsyncRead + Unpin,
        cap: usize,
    ) -> std::io::Result<Vec<u8>> {
        let mut result = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            let room = cap.saturating_sub(result.len());
            result.extend_from_slice(&buffer[..n.min(room)]);
        }
        Ok(result)
    }
    struct Readers {
        stdout: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
        stderr: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    }
    impl Drop for Readers {
        fn drop(&mut self) {
            self.stdout.abort();
            self.stderr.abort();
        }
    }
    let stream_cap = MAX_OUTPUT_BYTES.saturating_sub(1024) / 2;
    let mut readers = Readers {
        stdout: tokio::spawn(drain(stdout, stream_cap)),
        stderr: tokio::spawn(drain(stderr, stream_cap)),
    };
    let status = tokio::select! {
        _=ctx.cancellation.cancelled()=>Err(AgentError::Cancelled),
        _=tokio::time::sleep(Duration::from_secs(timeout_secs))=>Err(AgentError::Tool("command timed out".into())),
        status=child.0.wait()=>status.map_err(AgentError::Io),
    };
    // Also terminate descendants that retained output pipes after their parent exited.
    let _ = child.0.start_kill();
    let reads = tokio::time::timeout(Duration::from_secs(2), async {
        let (out, err) = tokio::join!(&mut readers.stdout, &mut readers.stderr);
        Ok::<_, AgentError>((
            out.map_err(|_| AgentError::Tool("stdout reader failed".into()))??,
            err.map_err(|_| AgentError::Tool("stderr reader failed".into()))??,
        ))
    })
    .await;
    let reads = match reads {
        Ok(value) => value?,
        Err(_) => {
            readers.stdout.abort();
            readers.stderr.abort();
            let _ = tokio::join!(&mut readers.stdout, &mut readers.stderr);
            return Err(AgentError::Tool("output pipes did not close".into()));
        }
    };
    let status = status?;
    Ok(bounded_text(format!(
        "Exit: {status}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&reads.0),
        String::from_utf8_lossy(&reads.1)
    )))
}
