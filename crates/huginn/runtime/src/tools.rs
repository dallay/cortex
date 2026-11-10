use async_trait::async_trait;
use huginn_core::{
    tools::{tool_registration_id, ToolRegistration, ToolRegistrationService},
    AgentError, ApprovalProvenance, ApprovalRequest, ApprovalResource, PreparedAction, Result,
    Tool, ToolCall, ToolContext, ToolDefinition, ToolRegistry,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

pub const MAX_FILE_BYTES: u64 = 1_048_576;
pub use huginn_application::output::{bounded_text, MAX_OUTPUT_BYTES};
#[derive(Default, Clone)]
pub struct Registry {
    tools: Arc<RwLock<BTreeMap<String, Arc<RegisteredTool>>>>,
}
struct RegisteredTool {
    inner: Arc<dyn Tool>,
    visible: std::sync::atomic::AtomicBool,
    revoked: huginn_core::CancellationToken,
    generation: u64,
    native_file_builtin: bool,
    name: String,
}
impl RegisteredTool {
    fn active(&self) -> bool {
        self.visible.load(std::sync::atomic::Ordering::Acquire) && !self.revoked.is_cancelled()
    }
}
#[async_trait]
impl Tool for RegisteredTool {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }
    async fn prepare(&self, call: &ToolCall, ctx: &ToolContext) -> Result<PreparedAction> {
        if !self.active() {
            return Err(AgentError::Tool("tool registration revoked".into()));
        }
        tokio::select! {
            biased;
            _ = self.revoked.cancelled() => Err(AgentError::Cancelled),
            result = self.inner.prepare(call, ctx) => result.map(|mut prepared| {
                if let Some(request) = prepared.approval.as_mut() {
                    request.provenance = Some(ApprovalProvenance::Untrusted);
                    request.resource = None;
                    if self.native_file_builtin
                        && matches!(self.name.as_str(), "write_file" | "edit_file")
                        && request.action == format!("native.{}", self.name)
                    {
                        if let Some(path) = prepared.payload["resolved"].as_str().map(PathBuf::from) {
                            let requested_path = prepared.payload["path"]
                                .as_str()
                                .map(Path::new);
                            let root = ctx.workspace.canonicalize();
                            if path.is_absolute()
                                && root.as_ref().is_ok_and(|root| path.starts_with(root))
                            {
                                request.resource = Some(ApprovalResource {
                                    sensitive: requested_path
                                        .is_some_and(huginn_guards::sensitive)
                                        || huginn_guards::sensitive(&path),
                                    canonical_path: path,
                                });
                                request.provenance = Some(ApprovalProvenance::NativeFileBuiltin);
                            }
                        }
                    }
                }
                prepared
            }),
        }
    }
    async fn execute(&self, action: PreparedAction, ctx: &ToolContext) -> Result<String> {
        if !self.active() {
            return Err(AgentError::Tool("tool registration revoked".into()));
        }
        tokio::select! {
            biased;
            _ = self.revoked.cancelled() => Err(AgentError::Cancelled),
            result = self.inner.execute(action, ctx) => result,
        }
    }
}
pub struct ToolLease {
    registry: Registry,
    name: String,
    entry: Arc<RegisteredTool>,
}
impl Drop for ToolLease {
    fn drop(&mut self) {
        self.entry.revoked.cancel();
        let mut tools = self
            .registry
            .tools
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if tools
            .get(&self.name)
            .is_some_and(|entry| Arc::ptr_eq(entry, &self.entry))
        {
            tools.remove(&self.name);
        }
    }
}
impl Registry {
    /// Convenience for isolated adapter tests. Production uses NativeToolPlugin.
    pub fn native(timeout_secs: u64) -> Result<Self> {
        let registry = Self::default();
        for tool in native_tools(timeout_secs) {
            registry.insert(tool)?;
        }
        Ok(registry)
    }
    pub fn insert(&self, tool: Arc<dyn Tool>) -> Result<()> {
        self.insert_entry(tool, 0, true, false)?;
        Ok(())
    }
    fn insert_entry(
        &self,
        tool: Arc<dyn Tool>,
        generation: u64,
        visible: bool,
        native_file_builtin: bool,
    ) -> Result<Arc<RegisteredTool>> {
        let name = tool.definition().name;
        let mut tools = self
            .tools
            .write()
            .map_err(|_| AgentError::Tool("registry unavailable".into()))?;
        if tools.contains_key(&name) {
            return Err(AgentError::Tool(format!("duplicate tool {name}")));
        }
        let entry = Arc::new(RegisteredTool {
            name: name.clone(),
            native_file_builtin,
            inner: tool,
            visible: std::sync::atomic::AtomicBool::new(visible),
            revoked: huginn_core::CancellationToken::new(),
            generation,
        });
        tools.insert(name, entry.clone());
        drop(tools);
        Ok(entry)
    }
    /// Staged registration is invisible until the supervisor commits activation.
    pub fn register(
        &self,
        ctx: &mut huginn_core::kernel::PluginContext,
        tool: Arc<dyn Tool>,
    ) -> Result<()> {
        let registry = ctx.resolve::<Self>(&registry_id())?;
        if !Arc::ptr_eq(&self.tools, &registry.tools) {
            return Err(AgentError::Composition(
                "tool registry does not match declared dependency".into(),
            ));
        }
        self.register_batch(ctx, vec![tool])
    }
    pub(crate) fn register_batch(
        &self,
        ctx: &mut huginn_core::kernel::PluginContext,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Result<()> {
        let mut entries = vec![];
        for tool in tools {
            let name = tool.definition().name;
            let entry = self.insert_entry(tool, ctx.generation(), false, false)?;
            ctx.own_resource(ToolLease {
                registry: self.clone(),
                name,
                entry: entry.clone(),
            });
            entries.push(entry);
        }
        let registry = self.clone();
        ctx.on_commit(move || {
            let _guard = registry
                .tools
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for entry in entries {
                entry
                    .visible
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        });
        Ok(())
    }
    fn register_native_file_builtins(
        &self,
        ctx: &mut huginn_core::kernel::PluginContext,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Result<()> {
        let mut entries = vec![];
        for tool in tools {
            let name = tool.definition().name;
            if !matches!(name.as_str(), "write_file" | "edit_file") {
                return Err(AgentError::Composition(
                    "only native file mutation builtins can receive session-grant provenance"
                        .into(),
                ));
            }
            let entry = self.insert_entry(tool, ctx.generation(), false, true)?;
            ctx.own_resource(ToolLease {
                registry: self.clone(),
                name,
                entry: entry.clone(),
            });
            entries.push(entry);
        }
        let registry = self.clone();
        ctx.on_commit(move || {
            let _guard = registry
                .tools
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for entry in entries {
                entry
                    .visible
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        });
        Ok(())
    }
    /// Atomically publish an unscoped batch for the legacy standalone MCP API.
    pub(crate) fn batch(&self, tools: Vec<Arc<dyn Tool>>) -> Result<Vec<ToolLease>> {
        let mut leases = vec![];
        for tool in tools {
            let name = tool.definition().name;
            let entry = self.insert_entry(tool, 0, false, false)?;
            leases.push(ToolLease {
                registry: self.clone(),
                name,
                entry,
            });
        }
        let _guard = self
            .tools
            .write()
            .map_err(|_| AgentError::Tool("registry unavailable".into()))?;
        for lease in &leases {
            lease
                .entry
                .visible
                .store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(leases)
    }
    pub fn generation(&self, name: &str) -> Option<u64> {
        self.tools
            .read()
            .ok()?
            .get(name)
            .filter(|entry| entry.active())
            .map(|entry| entry.generation)
    }
}
impl ToolRegistry for Registry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .read()
            .map(|tools| {
                tools
                    .values()
                    .filter(|t| t.active())
                    .map(|t| t.definition())
                    .collect()
            })
            .unwrap_or_default()
    }
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools
            .read()
            .ok()?
            .get(name)
            .filter(|entry| entry.active())
            .map(|entry| entry.clone() as Arc<dyn Tool>)
    }
}
impl ToolRegistration for Registry {
    fn register(
        &self,
        ctx: &mut huginn_core::kernel::PluginContext,
        tool: Arc<dyn Tool>,
    ) -> Result<()> {
        self.register_batch(ctx, vec![tool])
    }
}
pub fn native_tools(timeout_secs: u64) -> Vec<Arc<dyn Tool>> {
    native_tools_configured(timeout_secs, None)
}

pub fn native_tools_with_guards(
    timeout_secs: u64,
    guards: Arc<huginn_guards::Guardrails>,
) -> Vec<Arc<dyn Tool>> {
    native_tools_configured(timeout_secs, Some(guards))
}

fn native_tools_configured(
    timeout_secs: u64,
    guards: Option<Arc<huginn_guards::Guardrails>>,
) -> Vec<Arc<dyn Tool>> {
    [
        Kind::List,
        Kind::Read,
        Kind::Search,
        Kind::Write,
        Kind::Edit,
        Kind::Shell,
    ]
    .into_iter()
    .map(|kind| {
        Arc::new(NativeTool {
            kind,
            timeout_secs,
            guards: guards.clone(),
        }) as Arc<dyn Tool>
    })
    .collect()
}

/// One capability and lease per native tool; no special bootstrap registration path.
pub struct NativeToolPlugin(pub Arc<dyn Tool>);
#[async_trait]
impl huginn_core::kernel::Plugin for NativeToolPlugin {
    fn manifest(&self) -> huginn_core::kernel::Manifest {
        huginn_core::kernel::Manifest {
            id: format!("native.{}", self.0.definition().name),
            provides: vec![],
            requires: vec![registry_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut huginn_core::kernel::PluginContext) -> Result<()> {
        let registry = ctx.resolve::<Registry>(&registry_id())?;
        registry.register(ctx, self.0.clone())
    }
}
pub(crate) struct NativeBuiltinToolPlugin(pub Vec<Arc<dyn Tool>>);
#[async_trait]
impl huginn_core::kernel::Plugin for NativeBuiltinToolPlugin {
    fn manifest(&self) -> huginn_core::kernel::Manifest {
        huginn_core::kernel::Manifest {
            id: "native.file-builtins".into(),
            provides: vec![],
            requires: vec![registry_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut huginn_core::kernel::PluginContext) -> Result<()> {
        let registry = ctx.resolve::<Registry>(&registry_id())?;
        registry.register_native_file_builtins(ctx, self.0.clone())
    }
}
pub fn registry_id() -> huginn_core::kernel::ServiceId {
    huginn_core::kernel::ServiceId::new("agent:tool-registrations", 1)
}
pub struct RegistryPlugin(pub Registry);
#[async_trait]
impl huginn_core::kernel::Plugin for RegistryPlugin {
    fn manifest(&self) -> huginn_core::kernel::Manifest {
        huginn_core::kernel::Manifest {
            id: "tools.registry".into(),
            provides: vec![
                registry_id(),
                huginn_application::composition::tools_id(),
                tool_registration_id(),
            ],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut huginn_core::kernel::PluginContext) -> Result<()> {
        ctx.provide(
            tool_registration_id(),
            ToolRegistrationService::new(Arc::new(self.0.clone())),
        )?;
        ctx.provide(
            huginn_application::composition::tools_id(),
            huginn_core::ToolsService(Arc::new(self.0.clone())),
        )?;
        ctx.provide(registry_id(), self.0.clone())
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

fn skipped(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".pnpm-store" | ".cache"
    )
}
pub fn workspace_files(root: &Path, path: &Path) -> Result<Vec<PathBuf>> {
    workspace_files_guarded(root, path, None)
}
fn workspace_files_guarded(
    root: &Path,
    path: &Path,
    guards: Option<&huginn_guards::Guardrails>,
) -> Result<Vec<PathBuf>> {
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
        if guards.is_some_and(|policy| policy.check_path(root, &path).is_err()) {
            continue;
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
    guards: Option<Arc<huginn_guards::Guardrails>>,
}
#[async_trait]
impl Tool for NativeTool {
    fn definition(&self) -> ToolDefinition {
        let (name, description, properties, required) = match self.kind {
            Kind::List => ("list_files", "List up to 100 regular files recursively within a workspace path; narrow the path for larger trees (skips build/dependency directories).", json!({"path":{"type":"string"}}), vec![]),
            Kind::Read => ("read_file", "Read a UTF-8 file; use start_line/end_line for a bounded range.", json!({"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1}}), vec!["path"]),
            Kind::Search => ("search_files", "Search literal text in UTF-8 files under a workspace path.", json!({"path":{"type":"string"},"query":{"type":"string","minLength":1}}), vec!["query"]),
            Kind::Write => ("write_file", "Create or replace a UTF-8 file. Shows a diff and requires approval. Parent directory must exist.", json!({"path":{"type":"string"},"content":{"type":"string"}}), vec!["path", "content"]),
            Kind::Edit => ("edit_file", "Replace exactly one occurrence of old_text with new_text. Shows a diff and requires approval.", json!({"path":{"type":"string"},"old_text":{"type":"string","minLength":1},"new_text":{"type":"string"}}), vec!["path", "old_text", "new_text"]),
            Kind::Shell => ("shell", "Run a POSIX shell command in the workspace, with explicit approval and bounded time/output. This is not sandboxed.", json!({"command":{"type":"string","minLength":1}}), vec!["command"]),
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
            if let Some(guards) = &self.guards {
                guards
                    .check_shell_in(&ctx.workspace, command)
                    .map_err(|e| AgentError::Tool(e.into()))?;
            }
            if command.trim().is_empty() || command.len() > 16_384 {
                return Err(AgentError::Tool("invalid command length".into()));
            }
            return Ok(PreparedAction {
                approval: Some(ApprovalRequest {
                    id: call.id.clone(),
                    action: "native.shell".into(),
                    preview: format!("Directory: {}\nCommand: {command}", ctx.workspace.display()),
                    resource: None,
                    provenance: None,
                }),
                payload: call.arguments.clone(),
            });
        }
        let path = call.arguments["path"].as_str().unwrap_or(".");
        if let Some(guards) = &self.guards {
            guards
                .check_path(&ctx.workspace, Path::new(path))
                .map_err(|e| AgentError::Tool(e.into()))?;
        }
        if matches!(self.kind, Kind::Read | Kind::Write | Kind::Edit) {
            argument(&call.arguments, "path")?;
        }
        let resolved = resolve_path(&ctx.workspace, path, matches!(self.kind, Kind::Write))?;
        if let Some(guards) = &self.guards {
            guards
                .check_path(&ctx.workspace, &resolved)
                .map_err(|e| AgentError::Tool(e.into()))?;
        }
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
                    // resolve_path confines preview read to workspace; stable-tree guarantee only.
                    // Evidence: docs/security/semgrep-path-exceptions.md#runtime-prepare
                    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
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
                    resource: None,
                    provenance: None,
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
            if let Some(guards) = &self.guards {
                guards
                    .check_shell_in(&ctx.workspace, argument(&args, "command")?)
                    .map_err(|e| AgentError::Tool(e.into()))?;
            }
            return run_shell(argument(&args, "command")?, ctx, self.timeout_secs).await;
        }
        let raw = args["path"].as_str().unwrap_or(".");
        let path = resolve_path(&ctx.workspace, raw, matches!(self.kind, Kind::Write))?;
        if let Some(guards) = &self.guards {
            guards
                .check_path(&ctx.workspace, Path::new(raw))
                .map_err(|e| AgentError::Tool(e.into()))?;
            guards
                .check_path(&ctx.workspace, &path)
                .map_err(|e| AgentError::Tool(e.into()))?;
        }
        if json!(path) != args["resolved"] {
            return Err(AgentError::Tool(
                "path changed after preparation; request a new approval".into(),
            ));
        }
        match self.kind {
            Kind::Write | Kind::Edit => execute_mutation(&args, &path).await,
            Kind::Read => execute_read(&args, &path).await,
            Kind::List => execute_list(&path, ctx, self.guards.clone()).await,
            Kind::Search => execute_search(&args, &path, ctx, self.guards.clone()).await,
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
    // Private caller revalidates workspace path before stale check; stable-tree only.
    // Evidence: docs/security/semgrep-path-exceptions.md#runtime-mutation
    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
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
    // Private caller revalidates workspace path before read; stable-tree only.
    // Evidence: docs/security/semgrep-path-exceptions.md#runtime-read
    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
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

async fn workspace_file_list(
    path: &Path,
    ctx: &ToolContext,
    guards: Option<Arc<huginn_guards::Guardrails>>,
) -> Result<Vec<PathBuf>> {
    let root = ctx.workspace.clone();
    let input = path.to_path_buf();
    tokio::task::spawn_blocking(move || workspace_files_guarded(&root, &input, guards.as_deref()))
        .await
        .map_err(|_| AgentError::Tool("file traversal failed".into()))?
}

async fn execute_list(
    path: &Path,
    ctx: &ToolContext,
    guards: Option<Arc<huginn_guards::Guardrails>>,
) -> Result<String> {
    let files = workspace_file_list(path, ctx, guards).await?;
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

async fn execute_search(
    args: &Value,
    path: &Path,
    ctx: &ToolContext,
    guards: Option<Arc<huginn_guards::Guardrails>>,
) -> Result<String> {
    let files = workspace_file_list(path, ctx, guards.clone()).await?;
    let query = argument(args, "query")?;
    if query.is_empty() {
        return Err(AgentError::Tool("query must not be empty".into()));
    }
    let mut matches = vec![];
    for file in files {
        if ctx.cancellation.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        if guards
            .as_ref()
            .is_some_and(|policy| policy.check_path(&ctx.workspace, &file).is_err())
        {
            continue;
        }
        if tokio::fs::metadata(&file).await?.len() > MAX_FILE_BYTES {
            continue;
        }
        // Validated workspace root and regular non-link entries; stable-tree only.
        // Evidence: docs/security/semgrep-path-exceptions.md#runtime-search
        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
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
    #[cfg(target_os = "macos")]
    {
        let roots = crate::seatbelt::prepare_workspace(&ctx.workspace).map_err(|error| {
            AgentError::Tool(format!("cannot prepare shell workspace: {error:?}"))
        })?;
        let profile = crate::seatbelt::build_profile(&roots)
            .map_err(|error| AgentError::Tool(format!("cannot build shell profile: {error:?}")))?;
        return run_shell_with_seatbelt_profile(
            command,
            roots.workspace(),
            profile.as_str(),
            ctx,
            timeout_secs,
        )
        .await;
    }
    #[cfg(not(target_os = "macos"))]
    run_shell_with_launcher(
        "/bin/sh",
        vec!["-c".to_owned(), command.to_owned()],
        &ctx.workspace,
        ctx,
        timeout_secs,
    )
    .await
}

#[cfg(target_os = "macos")]
async fn run_shell_with_seatbelt_profile(
    command: &str,
    workspace: &Path,
    profile: &str,
    ctx: &ToolContext,
    timeout_secs: u64,
) -> Result<String> {
    let profile_file = create_seatbelt_profile(profile)?;
    let execution = run_shell_with_launcher(
        "/usr/bin/sandbox-exec",
        vec![
            "-f".to_owned(),
            profile_file.path().to_string_lossy().into_owned(),
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            command.to_owned(),
        ],
        workspace,
        ctx,
        timeout_secs,
    )
    .await;
    let cleanup = profile_file.cleanup();
    combine_shell_cleanup(execution, cleanup)
}

#[cfg(target_os = "macos")]
fn combine_shell_cleanup(
    execution: Result<String>,
    cleanup: std::io::Result<()>,
) -> Result<String> {
    match (execution, cleanup) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(AgentError::Tool(format!(
            "shell command may have run, but seatbelt profile cleanup failed: {cleanup_error}"
        ))),
        (Err(primary_error), Err(cleanup_error)) => Err(AgentError::Tool(format!(
            "shell execution failed ({primary_error}); the shell may have run; seatbelt profile cleanup also failed: {cleanup_error}"
        ))),
    }
}

#[cfg(target_os = "macos")]
fn cleanup_before_spawn(
    phase: &str,
    primary_error: std::io::Error,
    cleanup: impl IntoIterator<Item = std::io::Result<()>>,
) -> AgentError {
    let cleanup_errors = cleanup
        .into_iter()
        .filter_map(std::result::Result::err)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if cleanup_errors.is_empty() {
        AgentError::Tool(format!(
            "seatbelt profile {phase} failed before launcher spawn: {primary_error}"
        ))
    } else {
        AgentError::Tool(format!(
            "seatbelt profile {phase} failed before launcher spawn: {primary_error}; cleanup also failed: {}",
            cleanup_errors.join("; ")
        ))
    }
}

async fn run_shell_with_launcher(
    launcher: &str,
    launcher_args: Vec<String>,
    workspace: &Path,
    ctx: &ToolContext,
    timeout_secs: u64,
) -> Result<String> {
    use process_wrap::tokio::{CommandWrap, KillOnDrop, ProcessGroup};
    use tokio::{io::AsyncReadExt, process::Command};
    let mut cmd = Command::new(launcher);
    cmd.args(launcher_args)
        .current_dir(workspace)
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

#[cfg(target_os = "macos")]
struct SeatbeltProfileFile {
    directory: PathBuf,
    path: PathBuf,
}

#[cfg(target_os = "macos")]
impl SeatbeltProfileFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn cleanup(self) -> std::io::Result<()> {
        std::fs::remove_file(&self.path)?;
        std::fs::remove_dir(&self.directory)
    }
}

#[cfg(target_os = "macos")]
impl Drop for SeatbeltProfileFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[cfg(target_os = "macos")]
fn create_seatbelt_profile(profile: &str) -> Result<SeatbeltProfileFile> {
    create_seatbelt_profile_under(profile, &std::env::temp_dir())
}

#[cfg(target_os = "macos")]
fn create_seatbelt_profile_under(profile: &str, temp_root: &Path) -> Result<SeatbeltProfileFile> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    const MAX_PROFILE_BYTES: usize = 64 * 1024;
    if profile.len() > MAX_PROFILE_BYTES {
        return Err(AgentError::Tool("shell profile exceeds size limit".into()));
    }
    let directory = temp_root.join(format!("huginn-seatbelt-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|error| {
            AgentError::Tool(format!(
                "seatbelt profile create failed before launcher spawn: {error}"
            ))
        })?;
    let path = directory.join("profile.sb");
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) => {
            let cleanup = std::fs::remove_dir(&directory);
            return Err(cleanup_before_spawn("open", error, [cleanup]));
        }
    };
    if let Err(error) = write_seatbelt_profile(&mut file, profile.as_bytes()) {
        drop(file);
        let cleanup = [std::fs::remove_file(&path), std::fs::remove_dir(&directory)];
        return Err(cleanup_before_spawn("write", error, cleanup));
    }
    drop(file);
    Ok(SeatbeltProfileFile { directory, path })
}

#[cfg(target_os = "macos")]
fn write_seatbelt_profile(
    writer: &mut impl std::io::Write,
    contents: &[u8],
) -> std::io::Result<()> {
    writer.write_all(contents)
}

#[cfg(all(test, target_os = "macos"))]
mod run_shell_seatbelt_tests {
    use super::{run_shell, run_shell_with_seatbelt_profile};
    use huginn_core::{CancellationToken, ToolContext};
    use std::path::PathBuf;
    use std::time::Duration;

    fn context(workspace: PathBuf) -> ToolContext {
        ToolContext {
            workspace,
            selected_skills: vec![],
            cancellation: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn run_shell_cannot_read_marker_outside_workspace() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let marker = fixture.path().join("outside-marker");
        std::fs::write(&marker, "outside-secret-marker").unwrap();
        assert!(marker.is_file(), "outside marker fixture must exist");
        let command = format!(
            "printf shell-started; LC_ALL=C /bin/cat '{}'",
            marker.display()
        );

        let output = run_shell(&command, &context(workspace), 10).await.unwrap();

        assert!(
            output.contains("shell-started"),
            "shell did not reach the command sentinel: {output}"
        );
        assert!(
            output.contains(&format!(
                "cat: {}: Operation not permitted",
                marker.display()
            )),
            "expected cat to report the outside-file denial: {output}"
        );
        let exit = output
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("Exit: exit status: "))
            .expect("run_shell must report a normal shell exit status, not a signal");
        assert_ne!(exit, "0", "outside marker was readable: {output}");
        assert!(
            !output.contains("outside-secret-marker"),
            "outside marker content leaked: {output}"
        );
    }

    // These are characterization tests: the current profile already denies these
    // paths. Each fixture is independent of real user data and is verified on the host.
    fn assert_external_read_denied(output: &str, marker: &std::path::Path, secret: &str) {
        assert!(
            output.contains("shell-started"),
            "shell sentinel missing: {output}"
        );
        assert!(
            output.contains(&format!(
                "cat: {}: Operation not permitted",
                marker.display()
            )),
            "expected /bin/cat's OS denial for the existing fixture: {output}"
        );
        let status = output
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("Exit: exit status: "))
            .expect("shell must return a normal status, not a signal");
        assert_ne!(status, "0", "external fixture was readable: {output}");
        assert!(!output.contains(secret), "fixture secret leaked: {output}");
    }

    #[tokio::test]
    async fn run_shell_cannot_read_file_in_another_workspace() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let workspace = first.path().join("workspace");
        let other_workspace = second.path().join("other-workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&other_workspace).unwrap();
        let marker = other_workspace.join("other-workspace-secret");
        let secret = "other-workspace-secret-value";
        std::fs::write(&marker, secret).unwrap();
        let marker = marker.canonicalize().unwrap();
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), secret);

        let output = run_shell(
            &format!(
                "printf 'shell-started\\n'; LC_ALL=C /bin/cat '{}'",
                marker.display()
            ),
            &context(workspace.canonicalize().unwrap()),
            10,
        )
        .await
        .unwrap();

        assert_external_read_denied(&output, &marker, secret);
    }

    #[tokio::test]
    async fn run_shell_cannot_read_credential_fixture_representing_home() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let home = fixture.path().join("isolated-home-fixture");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir_all(home.join(".config")).unwrap();
        let marker = home.join(".config/provider-credentials");
        let secret = "credential-fixture-secret-not-real";
        std::fs::write(&marker, secret).unwrap();
        let marker = marker.canonicalize().unwrap();
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), secret);

        let output = run_shell(
            &format!(
                "printf 'shell-started\\n'; LC_ALL=C /bin/cat '{}'",
                marker.display()
            ),
            &context(workspace.canonicalize().unwrap()),
            10,
        )
        .await
        .unwrap();

        assert_external_read_denied(&output, &marker, secret);
    }

    #[tokio::test]
    async fn run_shell_cannot_read_external_non_designated_path() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let external = fixture.path().join("external");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&external).unwrap();
        let marker = external.join("external-secret");
        let secret = "external-fixture-secret";
        std::fs::write(&marker, secret).unwrap();
        let marker = marker.canonicalize().unwrap();
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), secret);

        let output = run_shell(
            &format!(
                "printf 'shell-started\\n'; LC_ALL=C /bin/cat '{}'",
                marker.display()
            ),
            &context(workspace.canonicalize().unwrap()),
            10,
        )
        .await
        .unwrap();

        assert_external_read_denied(&output, &marker, secret);
    }

    #[tokio::test]
    async fn run_shell_cannot_read_unassigned_temp_fixture() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let unassigned_temp = fixture.path().join("unassigned-temp");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&unassigned_temp).unwrap();
        let marker = unassigned_temp.join("temp-secret");
        let secret = "unassigned-temp-fixture-secret";
        std::fs::write(&marker, secret).unwrap();
        let marker = marker.canonicalize().unwrap();
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), secret);

        let output = run_shell(
            &format!(
                "printf 'shell-started\\n'; LC_ALL=C /bin/cat '{}'",
                marker.display()
            ),
            &context(workspace.canonicalize().unwrap()),
            10,
        )
        .await
        .unwrap();

        assert_external_read_denied(&output, &marker, secret);
    }

    #[tokio::test]
    async fn run_shell_cannot_read_external_marker_after_changing_cwd() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let external = fixture.path().join("external-cwd");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&external).unwrap();
        let marker = external.join("cwd-secret");
        let secret = "cwd-does-not-expand-permissions";
        std::fs::write(&marker, secret).unwrap();
        let external = external.canonicalize().unwrap();
        let marker = marker.canonicalize().unwrap();
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), secret);

        let output = run_shell(
            &format!(
                "printf 'shell-started\\n'; if cd '{}' 2>cd-error; then printf 'cwd-changed\\n'; LC_ALL=C /bin/cat 'cwd-secret'; else printf 'cwd-denied\\n'; cat cd-error >&2; fi",
                external.display()
            ),
            &context(workspace.canonicalize().unwrap()),
            10,
        )
            .await
            .unwrap();

        assert!(
            output.contains("shell-started"),
            "shell sentinel missing: {output}"
        );
        if output.contains("cwd-changed") {
            assert!(
                output.contains("cat: cwd-secret: Operation not permitted"),
                "changing cwd exposed the external file or failed ambiguously: {output}"
            );
            let status = output
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("Exit: exit status: "))
                .expect("shell must return a normal status, not a signal");
            assert_ne!(
                status, "0",
                "external file was readable after cwd change: {output}"
            );
            assert!(
                !output.contains(secret),
                "external fixture secret leaked: {output}"
            );
        } else {
            assert!(
                output.contains("cwd-denied"),
                "cwd result is ambiguous: {output}"
            );
            assert!(
                output.contains("Operation not permitted"),
                "cwd change failed without an OS denial diagnostic: {output}"
            );
        }
        assert!(
            !output.contains(secret),
            "external fixture secret leaked: {output}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_shell_cannot_read_external_target_through_workspace_symlink() {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let external = fixture.path().join("external");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&external).unwrap();
        let marker = external.join("symlink-target-secret");
        let secret = "symlink-target-fixture-secret";
        std::fs::write(&marker, secret).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let marker = marker.canonicalize().unwrap();
        let link = workspace.join("external-link");
        symlink(&marker, &link).unwrap();
        assert_eq!(std::fs::read_to_string(&link).unwrap(), secret);
        assert_eq!(
            link.canonicalize().unwrap(),
            marker,
            "link must target external fixture"
        );

        let output = run_shell(
            &format!(
                "printf 'shell-started\\n'; LC_ALL=C /bin/cat '{}'",
                link.display()
            ),
            &context(workspace),
            10,
        )
        .await
        .unwrap();

        assert_external_read_denied(&output, &link, secret);
    }

    #[tokio::test]
    async fn run_shell_can_read_and_write_inside_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let canonical_workspace = workspace.path().canonicalize().unwrap();
        let marker = canonical_workspace.join("inside-marker");
        std::fs::write(&marker, "before").unwrap();
        let command = format!(
            "cat '{}' && printf after > '{}' && cat '{}'",
            marker.display(),
            marker.display(),
            marker.display()
        );

        let output = run_shell(&command, &context(canonical_workspace), 10)
            .await
            .unwrap();

        assert!(output.starts_with("Exit: exit status: 0\n"), "{output}");
        assert!(output.contains("beforeafter"), "{output}");
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "after");
    }

    fn run_host_probe(command: &str) -> std::process::Output {
        std::process::Command::new("/bin/sh")
            .args(["-c", command])
            .output()
            .expect("host control shell must start")
    }

    fn wait_for_listener_result<T>(receiver: std::sync::mpsc::Receiver<T>, description: &str) -> T {
        receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| panic!("timed out waiting for {description}"))
    }

    fn assert_normal_nonzero(output: &str, protocol: &str) {
        let status = output
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("Exit: exit status: "))
            .unwrap_or_else(|| {
                panic!("{protocol} probe did not return a normal shell status: {output}")
            });
        assert_ne!(
            status, "0",
            "{protocol} probe unexpectedly succeeded: {output}"
        );
        assert!(output.contains("nc:") && output.contains("Operation not permitted"),
                "{protocol} probe lacks an nc OS-denial diagnostic (possible harness/binary failure): {output}");
    }

    #[tokio::test]
    async fn run_shell_seatbelt_denies_tcp_connect_after_host_control_succeeds() {
        use std::net::TcpListener;
        use std::sync::mpsc;
        use std::thread;
        use std::time::Instant;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        let acceptor = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = sender.send(true);
                        drop(stream);
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            let _ = sender.send(false);
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("TCP listener failed: {error}"),
                }
            }
        });
        let probe = format!("/usr/bin/nc -v -w 1 127.0.0.1 {}", address.port());
        let host = run_host_probe(&probe);
        assert!(host.status.success(), "TCP host control failed: {host:?}");
        assert!(
            wait_for_listener_result(receiver, "host TCP connection"),
            "host control did not connect"
        );
        acceptor.join().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let sandbox_address = listener.local_addr().unwrap();
        let sandbox_probe = format!("/usr/bin/nc -v -w 1 127.0.0.1 {}", sandbox_address.port());
        let (sender, receiver) = mpsc::channel();
        listener.set_nonblocking(true).unwrap();
        let acceptor = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match listener.accept() {
                    Ok(_) => {
                        let _ = sender.send(true);
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            let _ = sender.send(false);
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("TCP listener failed: {error}"),
                }
            }
        });
        let workspace = tempfile::tempdir().unwrap();
        let output = run_shell(
            &format!("printf 'shell-started\\n'; {sandbox_probe}"),
            &context(workspace.path().canonicalize().unwrap()),
            5,
        )
        .await
        .unwrap();
        let sandbox_accepted = wait_for_listener_result(receiver, "sandboxed TCP accept");
        assert!(
            output.contains("shell-started"),
            "shell sentinel missing (listener accepted={sandbox_accepted}): {output}"
        );
        assert!(
            !sandbox_accepted,
            "TCP connection reached host listener: {output}"
        );
        assert_normal_nonzero(&output, "TCP");
        acceptor.join().unwrap();
    }

    #[tokio::test]
    async fn run_shell_seatbelt_denies_udp_datagram_after_host_control_succeeds() {
        use std::net::UdpSocket;
        use std::sync::mpsc;
        use std::thread;

        let receiver_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver_socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let address = receiver_socket.local_addr().unwrap();
        let (sender, received) = mpsc::channel();
        let receiver_thread = thread::spawn(move || {
            let mut buffer = [0; 64];
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut messages = Vec::new();
            while std::time::Instant::now() < deadline {
                match receiver_socket.recv_from(&mut buffer) {
                    Ok((length, _)) => {
                        let message = buffer[..length].to_vec();
                        messages.push(message.clone());
                        if message == b"udp-probe" {
                            break;
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(error) => panic!("UDP listener failed: {error}"),
                }
            }
            let _ = sender.send(messages);
        });
        let probe = format!(
            "printf udp-probe | /usr/bin/nc -v -u -w 1 127.0.0.1 {}",
            address.port()
        );
        let host = run_host_probe(&probe);
        assert!(host.status.success(), "UDP host control failed: {host:?}");
        let host_messages = wait_for_listener_result(received, "host UDP datagrams");
        assert!(
            host_messages.iter().any(|message| message == b"udp-probe"),
            "host control did not deliver the payload; received {host_messages:?}"
        );
        receiver_thread.join().unwrap();

        let receiver_socket = UdpSocket::bind(address).unwrap();
        receiver_socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let (sender, received) = mpsc::channel();
        let receiver_thread = thread::spawn(move || {
            let mut buffer = [0; 64];
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut messages = Vec::new();
            while std::time::Instant::now() < deadline {
                match receiver_socket.recv_from(&mut buffer) {
                    Ok((length, _)) => {
                        let message = buffer[..length].to_vec();
                        messages.push(message.clone());
                        if message == b"udp-probe" {
                            break;
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(error) => panic!("UDP listener failed: {error}"),
                }
            }
            let _ = sender.send(messages);
        });
        let workspace = tempfile::tempdir().unwrap();
        let output = run_shell(
            &format!("printf 'shell-started\\n'; {probe}"),
            &context(workspace.path().canonicalize().unwrap()),
            5,
        )
        .await
        .unwrap();
        assert!(
            output.contains("shell-started"),
            "shell sentinel missing: {output}"
        );
        let sandbox_messages = wait_for_listener_result(received, "sandboxed UDP datagrams");
        assert_normal_nonzero(&output, "UDP");
        assert!(
            sandbox_messages.is_empty(),
            "sandboxed UDP probe sent datagrams to the host listener: {sandbox_messages:?}"
        );
        receiver_thread.join().unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_shell_seatbelt_denies_unix_socket_connect_after_host_control_succeeds() {
        use std::os::unix::net::UnixListener;
        use std::sync::mpsc;
        use std::thread;
        use std::time::Instant;

        let workspace = tempfile::tempdir().unwrap();
        let workspace_path = workspace.path().canonicalize().unwrap();
        let socket_path = workspace_path.join("probe.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let path = socket_path.clone();
        let (sender, receiver) = mpsc::channel();
        let acceptor = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                match listener.accept() {
                    Ok(_) => {
                        let _ = sender.send(true);
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            let _ = sender.send(false);
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("Unix listener failed: {error}"),
                }
            }
        });
        let probe = format!("/usr/bin/nc -v -U '{}' -w 1", path.display());
        let host = run_host_probe(&probe);
        assert!(
            host.status.success(),
            "Unix socket host control failed: {host:?}"
        );
        assert!(
            wait_for_listener_result(receiver, "host Unix connection"),
            "host control did not connect"
        );
        acceptor.join().unwrap();
        std::fs::remove_file(&socket_path).unwrap();

        let listener = UnixListener::bind(&socket_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let (sender, receiver) = mpsc::channel();
        let acceptor = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match listener.accept() {
                    Ok(_) => {
                        let _ = sender.send(true);
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            let _ = sender.send(false);
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("Unix listener failed: {error}"),
                }
            }
        });
        let output = run_shell(
            &format!("printf 'shell-started\\n'; if [ -S '{}' ]; then printf 'socket-visible\\n'; else printf 'socket-invisible\\n'; fi; {probe}; status=$?; printf 'nc-status=%s\\n' \"$status\"; exit \"$status\"", path.display()),
            &context(workspace_path),
            5,
        )
            .await
            .unwrap();
        let sandbox_accepted = wait_for_listener_result(receiver, "sandboxed Unix connection");
        assert!(
            output.contains("shell-started"),
            "shell sentinel missing: {output}"
        );
        assert!(
            output.contains("socket-visible"),
            "Unix socket path is not visible under the sandbox: {output}"
        );
        assert!(
            output.starts_with("Exit: exit status: 1\n") && output.contains("nc-status=1\n"),
            "the sandboxed nc command must return a normal nonzero status: {output}"
        );
        // Unlike TCP/UDP, this macOS nc build does not print a connect diagnostic for a
        // denied Unix-domain socket. The host control connects to this exact socket, the
        // sandboxed shell confirms the socket path is visible, and the same nc command
        // returns nonzero while the listener accepts no connection.
        assert!(!sandbox_accepted, "Unix connection reached host listener");
        acceptor.join().unwrap();
    }

    #[tokio::test]
    async fn run_shell_preserves_cwd_selected_path_null_stdin_and_separate_streams() {
        let workspace = tempfile::tempdir().unwrap();
        let canonical_workspace = workspace.path().canonicalize().unwrap();
        let command = "printf 'cwd=%s\\n' \"$PWD\"; printf 'path=%s\\n' \"$PATH\"; printf 'shell=%s\\n' \"$0\"; printf 'home=%s\\n' \"${HOME-unset}\"; if read value; then printf 'stdin=%s\\n' \"$value\"; else printf 'stdin=eof\\n'; fi; printf 'out-marker\\n'; printf 'err-marker\\n' >&2";

        let output = run_shell(command, &context(canonical_workspace.clone()), 10)
            .await
            .unwrap();

        assert!(output.contains(&format!("cwd={}\n", canonical_workspace.display())));
        assert!(output.contains(&format!(
            "path={}\n",
            std::env::var("PATH").unwrap_or_default()
        )));
        assert!(output.contains("shell=/bin/sh\n"));
        if std::env::var_os("HOME").is_some() {
            assert!(output.contains("home=unset\n"));
        }
        assert!(output.contains("stdin=eof\n"));
        let stdout = output
            .split("stdout:\n")
            .nth(1)
            .unwrap()
            .split("\nstderr:")
            .next()
            .unwrap();
        let stderr = output.split("stderr:\n").nth(1).unwrap();
        assert!(stdout.contains("out-marker\n"));
        assert!(stderr.contains("err-marker\n"));
        assert!(!stdout.contains("err-marker"));
        assert!(!stderr.contains("out-marker"));
    }

    #[tokio::test]
    async fn run_shell_bounds_captured_output_while_draining_both_streams() {
        let workspace = tempfile::tempdir().unwrap();
        let output = run_shell(
            "i=0; while [ $i -lt 100000 ]; do printf O; i=$((i+1)); done; i=0; while [ $i -lt 100000 ]; do printf E >&2; i=$((i+1)); done",
            &context(workspace.path().canonicalize().unwrap()),
            10,
        )
            .await
            .unwrap();

        assert!(output.len() <= super::MAX_OUTPUT_BYTES);
        assert!(output.contains("stdout:\nO"));
        assert!(output.contains("stderr:\nE"));
    }

    #[tokio::test]
    async fn invalid_workspace_fails_before_command_can_write_marker() {
        let fixture = tempfile::tempdir().unwrap();
        let marker = fixture.path().join("must-not-exist");
        let missing_workspace = fixture.path().join("missing-workspace");

        let result = run_shell(
            &format!("printf reached > '{}'", marker.display()),
            &context(missing_workspace),
            10,
        )
        .await;

        assert!(result.is_err());
        assert!(!marker.exists(), "shell started despite workspace failure");
    }

    #[tokio::test]
    async fn sandbox_exec_rejects_invalid_profile_before_shell_can_write_marker() {
        let fixture = tempfile::tempdir().unwrap();
        let profile = fixture.path().join("invalid.sb");
        let marker = fixture.path().join("must-not-exist");
        std::fs::write(&profile, "this is not valid SBPL\n").unwrap();

        let output = std::process::Command::new("/usr/bin/sandbox-exec")
            .arg("-f")
            .arg(&profile)
            .args(["/bin/sh", "-c"])
            .arg(format!("printf reached > '{}'", marker.display()))
            .output()
            .expect("macOS must provide /usr/bin/sandbox-exec");

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(65),
            "unexpected rejection: {output:?}"
        );
        assert!(
            stderr.contains("unbound variable") && stderr.contains("invalid.sb"),
            "sandbox-exec did not identify the invalid profile and parse error: {stderr}"
        );
        assert!(!marker.exists(), "shell ran despite profile rejection");
    }

    #[tokio::test]
    async fn run_shell_reports_seatbelt_profile_rejection_without_running_shell() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_path = std::fs::canonicalize(workspace.path()).unwrap();
        let marker = workspace.path().join("invalid-profile-shell-ran");
        let ctx = context(workspace_path.clone());

        let result = run_shell_with_seatbelt_profile(
            &format!("printf executed > '{}'", marker.display()),
            &workspace_path,
            "this is not valid SBPL\n",
            &ctx,
            5,
        )
        .await;

        let output = result.expect("launcher status and diagnostic must remain observable");
        assert!(output.starts_with("Exit: exit status: 65\n"), "{output}");
        assert!(
            output.contains("unbound variable") && output.contains("profile.sb"),
            "expected the observed sandbox parser diagnostic: {output}"
        );
        assert!(
            !marker.exists(),
            "shell must not execute after profile rejection"
        );
    }

    #[tokio::test]
    async fn run_shell_preserves_shell_exit_code_65_with_valid_seatbelt_profile() {
        let workspace = tempfile::tempdir().unwrap();
        let roots = crate::seatbelt::prepare_workspace(workspace.path()).unwrap();
        let profile = crate::seatbelt::build_profile(&roots).unwrap();
        let workspace_path = roots.workspace().to_path_buf();
        let ctx = context(workspace_path.clone());

        let result =
            run_shell_with_seatbelt_profile("exit 65", &workspace_path, profile.as_str(), &ctx, 5)
                .await;

        let output = result.expect("valid profile must preserve the shell's exit status");
        assert!(output.starts_with("Exit: exit status: 65\n"), "{output}");
    }

    #[tokio::test]
    async fn launcher_spawn_failure_does_not_start_shell_command() {
        let workspace = tempfile::tempdir().unwrap();
        let marker = workspace.path().join("spawn-marker");
        let ctx = context(workspace.path().canonicalize().unwrap());

        let result = super::run_shell_with_launcher(
            "/huginn-test-launcher-that-does-not-exist",
            vec![],
            &ctx.workspace,
            &ctx,
            5,
        )
        .await;

        assert!(result.is_err(), "missing launcher must be a spawn error");
        assert!(
            !marker.exists(),
            "command ran despite launcher spawn failure"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_executable_launcher_returns_spawn_error_without_running_command() {
        use std::os::unix::fs::PermissionsExt;

        let workspace = tempfile::tempdir().unwrap();
        let launcher = workspace.path().join("not-executable");
        std::fs::write(&launcher, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o600)).unwrap();
        let marker = workspace.path().join("non-executable-marker");
        let ctx = context(workspace.path().canonicalize().unwrap());
        let result = super::run_shell_with_launcher(
            launcher.to_str().unwrap(),
            vec![
                "-c".into(),
                format!("printf reached > '{}'", marker.display()),
            ],
            &ctx.workspace,
            &ctx,
            5,
        )
        .await;

        assert!(
            result.is_err(),
            "non-executable launcher must fail at spawn"
        );
        assert!(!marker.exists(), "command ran despite spawn failure");
    }

    #[test]
    fn profile_creation_failure_from_non_directory_parent_is_propagated() {
        let temp = tempfile::tempdir().unwrap();
        let parent_file = temp.path().join("not-a-directory");
        std::fs::write(&parent_file, "file").unwrap();
        let result = super::create_seatbelt_profile_under("(deny default)\n", &parent_file);
        let error = match result {
            Ok(_) => panic!("profile creation must fail"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("Not a directory"), "{error}");
        assert!(error.contains("before launcher spawn"), "{error}");
    }

    #[test]
    fn pre_spawn_error_composition_preserves_primary_and_cleanup_failures() {
        let error = super::cleanup_before_spawn(
            "write",
            std::io::Error::other("primary write failure"),
            [Err(std::io::Error::other("profile unlink failure"))],
        );
        let message = error.to_string();
        assert!(message.contains("primary write failure"), "{message}");
        assert!(message.contains("profile unlink failure"), "{message}");
        assert!(message.contains("before launcher spawn"), "{message}");
    }

    #[test]
    fn post_spawn_error_composition_preserves_primary_and_possible_execution() {
        let error = super::combine_shell_cleanup(
            Err(huginn_core::AgentError::Tool("command timed out".into())),
            Err(std::io::Error::other("profile cleanup failure")),
        )
        .expect_err("cleanup failure must remain observable");
        let message = error.to_string();
        assert!(message.contains("command timed out"), "{message}");
        assert!(message.contains("profile cleanup failure"), "{message}");
        assert!(message.contains("may have run"), "{message}");
    }

    #[test]
    fn post_spawn_success_with_cleanup_error_does_not_claim_shell_was_skipped() {
        let error = super::combine_shell_cleanup(
            Ok("shell output".into()),
            Err(std::io::Error::other("profile cleanup failure")),
        )
        .expect_err("cleanup failure must remain observable");
        let message = error.to_string();
        assert!(message.contains("profile cleanup failure"), "{message}");
        assert!(message.contains("may have run"), "{message}");
        assert!(!message.contains("did not run"), "{message}");
    }

    #[derive(Debug)]
    struct FailingWriter;

    impl std::io::Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn profile_write_helper_propagates_writer_failure() {
        let error = super::write_seatbelt_profile(&mut FailingWriter, b"profile")
            .expect_err("writer error must propagate");
        assert_eq!(error.to_string(), "injected write failure");
    }

    #[tokio::test]
    async fn oversized_profile_is_rejected_before_shell_can_write_marker() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_path = workspace.path().canonicalize().unwrap();
        let marker = workspace_path.join("oversized-profile-shell-ran");
        let ctx = context(workspace_path.clone());
        let oversized = "x".repeat(64 * 1024 + 1);

        let result = run_shell_with_seatbelt_profile(
            &format!("printf executed > '{}'", marker.display()),
            &workspace_path,
            &oversized,
            &ctx,
            5,
        )
        .await;

        assert!(result.is_err(), "oversized profile must be rejected");
        assert!(!marker.exists(), "shell ran with oversized profile");
    }

    #[test]
    fn profile_cleanup_propagates_directory_removal_error() {
        let temp = tempfile::tempdir().unwrap();
        let profile =
            super::create_seatbelt_profile_under("(deny default)\n", temp.path()).unwrap();
        let directory = profile.directory.clone();
        std::fs::write(directory.join("unexpected-entry"), "fixture").unwrap();

        let result = profile.cleanup();

        assert!(
            result.is_err(),
            "non-empty profile directory must report cleanup failure"
        );
        assert!(directory.exists(), "failed cleanup must not claim removal");
        std::fs::remove_file(directory.join("unexpected-entry")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn profile_cleanup_removes_profile_and_private_directory() {
        let temp = tempfile::tempdir().unwrap();
        let profile =
            super::create_seatbelt_profile_under("(deny default)\n", temp.path()).unwrap();
        let path = profile.path().to_path_buf();
        let directory = profile.directory.clone();

        profile.cleanup().unwrap();

        assert!(!path.exists());
        assert!(!directory.exists());
    }

    async fn wait_for_pid_file(path: &std::path::Path) -> Vec<i32> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(path) {
                    let pids = pid
                        .split_whitespace()
                        .filter_map(|pid| pid.parse().ok())
                        .collect::<Vec<i32>>();
                    if pids.len() == 2 {
                        return pids;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("descendant did not write its PID sentinel")
    }

    fn host_process_snapshot(pids: &[i32]) -> String {
        let ids = pids
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let output = std::process::Command::new("/bin/ps")
            .args(["-o", "pid=,ppid=,pgid=,stat=,command=", "-p", &ids])
            .output()
            .expect("test host must provide /bin/ps");
        assert!(
            output.status.success(),
            "host /bin/ps failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("host /bin/ps output must be UTF-8")
    }

    fn assert_live_process_rows(pids: &[i32], snapshot: &str) {
        let rows = snapshot
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                Some((
                    fields.next()?.parse::<i32>().ok()?,
                    fields.next()?.parse::<i32>().ok()?,
                    fields.next()?.parse::<i32>().ok()?,
                    fields.next()?.to_owned(),
                    fields.collect::<Vec<_>>().join(" "),
                ))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows.len(),
            2,
            "expected exactly shell and sleep rows, got {rows:?}; snapshot:\n{snapshot}"
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.0)
                .collect::<std::collections::BTreeSet<_>>(),
            pids.iter().copied().collect()
        );
        let shell = rows
            .iter()
            .find(|row| row.4.contains("sh -c"))
            .expect("host snapshot must identify the shell row");
        let sleep = rows
            .iter()
            .find(|row| row.4.contains("/bin/sleep 60"))
            .expect("host snapshot must identify the sleep row");
        assert_eq!(
            shell.2, shell.0,
            "shell PGID must equal the process-group leader PID; snapshot:\n{snapshot}"
        );
        assert_eq!(
            sleep.2, shell.0,
            "sleep PGID must equal shell/leader PID; snapshot:\n{snapshot}"
        );
    }

    fn assert_process_terminated(pid: i32, snapshot: &str) {
        // A zombie has terminated and cannot execute or retain its pipes. `kill(pid, 0)`
        // alone treats zombies as present, so inspect the process state as well.
        let probe = std::process::Command::new("/bin/ps")
            .args([
                "-o",
                "pid=,ppid=,pgid=,stat=,command=",
                "-p",
                &pid.to_string(),
            ])
            .output()
            .expect("test host must provide /bin/ps");
        let detail = String::from_utf8_lossy(&probe.stdout);
        let state = detail.split_whitespace().nth(3).unwrap_or("");
        assert!(
            !probe.status.success() || state.starts_with('Z'),
            "recorded process {pid} is still executable (ps row {:?}); live host snapshot:\n{snapshot}",
            detail.trim(),
        );
    }

    #[tokio::test]
    async fn run_shell_timeout_terminates_descendant_holding_output_pipes() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_path = workspace.path().canonicalize().unwrap();
        let pid_file = workspace_path.join("timeout-descendant.pid");
        let command = format!(
            "/bin/sleep 60 & child=$!; printf '%s %s' \"$$\" \"$child\" > '{}'; wait",
            pid_file.display()
        );
        let operation =
            tokio::spawn(async move { run_shell(&command, &context(workspace_path), 3).await });
        let pid = wait_for_pid_file(&pid_file).await;
        let snapshot = host_process_snapshot(&pid);
        eprintln!("timeout fixture process snapshot:\n{snapshot}");
        assert_live_process_rows(&pid, &snapshot);
        let result = tokio::time::timeout(Duration::from_secs(6), operation)
            .await
            .expect("timeout run_shell did not return")
            .expect("run_shell task panicked");
        assert!(
            matches!(&result, Err(huginn_core::AgentError::Tool(message)) if message == "command timed out"),
            "expected timeout result, got {result:?}"
        );
        for process in pid {
            assert_process_terminated(process, &snapshot);
        }
    }

    #[tokio::test]
    async fn run_shell_cancellation_terminates_descendant_holding_output_pipes() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_path = workspace.path().canonicalize().unwrap();
        let pid_file = workspace_path.join("cancel-descendant.pid");
        let cancellation = CancellationToken::new();
        let ctx = ToolContext {
            workspace: workspace_path,
            selected_skills: vec![],
            cancellation: cancellation.clone(),
        };
        let command = format!(
            "/bin/sleep 60 & child=$!; printf '%s %s' \"$$\" \"$child\" > '{}'; wait",
            pid_file.display()
        );
        let operation = tokio::spawn(async move { run_shell(&command, &ctx, 30).await });

        let pid = wait_for_pid_file(&pid_file).await;
        let snapshot = host_process_snapshot(&pid);
        eprintln!("cancellation fixture process snapshot:\n{snapshot}");
        assert_live_process_rows(&pid, &snapshot);
        cancellation.cancel();
        let result = tokio::time::timeout(Duration::from_secs(5), operation)
            .await
            .expect("cancelled run_shell did not return")
            .expect("run_shell task panicked");
        assert!(
            matches!(result, Err(huginn_core::AgentError::Cancelled)),
            "expected cancellation result, got {result:?}"
        );
        for process in pid {
            assert_process_terminated(process, &snapshot);
        }
    }
}
