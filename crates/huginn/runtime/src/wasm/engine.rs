use super::{
    WasmActivity, DEADLINE_SECS, FUEL, MAX_COMPONENT_BYTES, MAX_OUTPUT_BYTES, MAX_READ_BYTES,
    MAX_SOURCE_BYTES, MEMORY_BYTES, YIELD_FUEL,
};
use huginn_core::{AgentError, CancellationToken, Result};
use std::{
    io::Read,
    path::Path,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use wasmtime::{
    component::{Resource, ResourceTable},
    Engine,
};

pub(super) mod bindings {
    wasmtime::component::bindgen!({
        path: "../plugin-api/wit/experimental-rust-outline",
        world: "rust-outline",
        imports: { default: async | trappable },
        exports: { default: async },
        with: { "huginn:rust-outline/snapshots.snapshot": super::Snapshot },
    });
}
pub struct Snapshot {
    pub bytes: Vec<u8>,
    pub remaining: usize,
}
pub(super) struct State {
    pub table: ResourceTable,
    limits: wasmtime::StoreLimits,
    activity: WasmActivity,
}
impl Drop for State {
    fn drop(&mut self) {
        self.activity.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}
impl bindings::huginn::rust_outline::snapshots::Host for State {}
impl bindings::huginn::rust_outline::snapshots::HostSnapshot for State {
    async fn size(&mut self, resource: Resource<Snapshot>) -> wasmtime::Result<u32> {
        self.activity.0.imports.fetch_add(1, Ordering::AcqRel);
        Ok(u32::try_from(self.table.get(&resource)?.bytes.len())?)
    }
    async fn read(
        &mut self,
        resource: Resource<Snapshot>,
        offset: u32,
        length: u32,
    ) -> wasmtime::Result<std::result::Result<Vec<u8>, String>> {
        self.activity.0.imports.fetch_add(1, Ordering::AcqRel);
        let snapshot = self.table.get_mut(&resource)?;
        let end = (offset as usize).checked_add(length as usize);
        if length > MAX_READ_BYTES
            || length as usize > snapshot.remaining
            || end.is_none_or(|end| end > snapshot.bytes.len())
        {
            return Err(wasmtime::Error::msg(
                "snapshot read limit or range exceeded",
            ));
        }
        snapshot.remaining -= length as usize;
        Ok(Ok(snapshot.bytes[offset as usize..end.unwrap()].to_vec()))
    }
    async fn drop(&mut self, resource: Resource<Snapshot>) -> wasmtime::Result<()> {
        self.table.delete(resource)?;
        Ok(())
    }
}

pub(super) struct Runtime {
    pub engine: Engine,
    pub pre: bindings::RustOutlinePre<State>,
    pub activity: WasmActivity,
    slots: Arc<tokio::sync::Semaphore>,
}
impl Runtime {
    pub fn load(path: &Path) -> Result<Self> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        use cap_std::{
            ambient_authority,
            fs::{Dir, OpenOptions},
        };

        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| error("missing component filename"))?;
        let directory = Dir::open_ambient_dir(parent, ambient_authority())?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = directory.open_with(name, &options)?;
        if !file.metadata()?.is_file() {
            return Err(error("component must be a regular file"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_COMPONENT_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_COMPONENT_BYTES {
            return Err(error("component exceeds 16 MiB"));
        }
        let mut config = wasmtime::Config::new();
        config
            .consume_fuel(true)
            .max_wasm_stack(512 * 1024)
            .cranelift_opt_level(wasmtime::OptLevel::None);
        let engine = Engine::new(&config).map_err(|_| error("cannot initialize Wasmtime"))?;
        let component = wasmtime::component::Component::new(&engine, bytes)
            .map_err(|_| error("invalid WASM component"))?;
        let mut linker = wasmtime::component::Linker::new(&engine);
        bindings::RustOutline::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
            &mut linker,
            |state| state,
        )
        .map_err(|_| error("cannot link snapshot contract"))?;
        let pre = bindings::RustOutlinePre::new(linker.instantiate_pre(&component).map_err(|_| error("incompatible component imports; only huginn:rust-outline/snapshots@0.1.0 is available"))?).map_err(|_| error("incompatible outline export contract"))?;
        Ok(Self {
            engine,
            pre,
            activity: WasmActivity::default(),
            slots: Arc::new(tokio::sync::Semaphore::new(1)),
        })
    }

    pub async fn run(&self, bytes: Vec<u8>, cancel: &CancellationToken) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(DEADLINE_SECS);
        check_invocation(cancel, deadline)?;
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("component invocation already active"))?;
        let source = std::str::from_utf8(&bytes)
            .map_err(|_| error("source is not UTF-8"))?
            .to_owned();
        let limits = wasmtime::StoreLimitsBuilder::new()
            .memory_size(MEMORY_BYTES)
            .table_elements(10_000)
            .instances(16)
            .tables(4)
            .memories(2)
            .trap_on_grow_failure(true)
            .build();
        self.activity.0.active.fetch_add(1, Ordering::AcqRel);
        let state = State {
            table: ResourceTable::new(),
            limits,
            activity: self.activity.clone(),
        };
        let mut store = wasmtime::Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(FUEL).map_err(|_| error("cannot set fuel"))?;
        store
            .fuel_async_yield_interval(Some(YIELD_FUEL))
            .map_err(|_| error("cannot set async yield budget"))?;
        let resource = store
            .data_mut()
            .table
            .push(Snapshot {
                bytes,
                remaining: MAX_SOURCE_BYTES,
            })
            .map_err(|_| error("cannot grant snapshot"))?;
        let run = async {
            let instance = self
                .pre
                .instantiate_async(&mut store)
                .await
                .map_err(|_| error("component initialization trapped or exceeded limits"))?;
            let output = instance
                .call_outline(&mut store, resource)
                .await
                .map_err(|_| error("component execution trapped or exceeded limits"))?;
            validate_output(&output, &source, cancel, deadline).await?;
            check_invocation(cancel, deadline)?;
            Ok(output)
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AgentError::Cancelled),
            result = tokio::time::timeout_at(deadline, run) => result.map_err(|_| error("component invocation exceeded deadline"))?,
        }
        // All futures, stores, resources, and the concurrency permit are dropped
        // here, also when the registry revokes and drops this entire future.
    }
}

fn check_invocation(cancel: &CancellationToken, deadline: tokio::time::Instant) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(AgentError::Cancelled);
    }
    if tokio::time::Instant::now() >= deadline {
        return Err(error("component invocation exceeded deadline"));
    }
    Ok(())
}

async fn yield_validation(
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<()> {
    check_invocation(cancel, deadline)?;
    tokio::task::yield_now().await;
    check_invocation(cancel, deadline)
}

struct SourcePosition {
    offset: usize,
    line: usize,
    column: usize,
    content_start: usize,
}

async fn validate_output(
    output: &str,
    source: &str,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<()> {
    check_invocation(cancel, deadline)?;
    if output.len() > MAX_OUTPUT_BYTES {
        return Err(error("component output exceeds 48 KiB"));
    }
    let value: serde_json::Value =
        serde_json::from_str(output).map_err(|_| error("component output is not JSON"))?;
    let entries = value["declarations"]
        .as_array()
        .ok_or_else(|| error("missing declaration list"))?;
    let complete = value["complete"]
        .as_bool()
        .ok_or_else(|| error("missing completeness flag"))?;
    let diagnostics = value["diagnostics"]
        .as_array()
        .ok_or_else(|| error("missing diagnostics"))?;
    if entries.len() > 1_000
        || diagnostics.iter().any(|value| !value.is_string())
        || (!complete && diagnostics.is_empty())
    {
        return Err(error("invalid outline limits or diagnostics"));
    }
    let mut ranges = Vec::with_capacity(entries.len());
    let mut offsets = Vec::with_capacity(entries.len() * 2);
    // These are structural checks, not authentication of guest semantic metadata.
    for entry in entries {
        let start = entry["range"]["start_byte"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok());
        let end = entry["range"]["end_byte"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok());
        let (Some(start), Some(end)) = (start, end) else {
            return Err(error("invalid source range"));
        };
        let valid_range = start < end && source.get(start..end).is_some();
        if !valid_range
            || ["name", "signature"].iter().any(|field| {
                entry[field]
                    .as_str()
                    .is_none_or(|text| text.len() > MAX_OUTPUT_BYTES || text.trim().is_empty())
            })
            || !matches!(
                entry["kind"].as_str(),
                Some(
                    "function"
                        | "struct"
                        | "enum"
                        | "module"
                        | "trait"
                        | "impl"
                        | "method"
                        | "type"
                        | "const"
                        | "static"
                        | "union"
                )
            )
            || !(entry["container"].is_null() || entry["container"].is_string())
        {
            return Err(error("invalid declaration or source signature"));
        }
        ranges.push((start, end));
        offsets.extend([start, end]);
    }
    offsets.sort_unstable();
    offsets.dedup();
    let mut positions: Vec<_> = offsets
        .into_iter()
        .map(|offset| SourcePosition {
            offset,
            line: 1,
            column: 1,
            content_start: source.len(),
        })
        .collect();
    if let Some(last) = positions.last() {
        let last_offset = last.offset;
        let (mut next, mut pending) = (0, 0);
        let (mut line, mut column) = (1, 1);
        let mut next_yield = 16 * 1024;
        // Resolve only requested positions and leading whitespace in one pass.
        // Each position is updated once, even for duplicate or overlapping ranges.
        for (offset, character) in source[..last_offset]
            .char_indices()
            .map(|(offset, character)| (offset, Some(character)))
            .chain(std::iter::once((last_offset, None)))
        {
            if offset >= next_yield {
                yield_validation(cancel, deadline).await?;
                next_yield = offset + 16 * 1024;
            }
            while next < positions.len() && positions[next].offset == offset {
                positions[next].line = line;
                positions[next].column = column;
                next += 1;
            }
            if let Some(character) = character {
                if !character.is_whitespace() {
                    for position in &mut positions[pending..next] {
                        position.content_start = offset;
                    }
                    pending = next;
                }
                if character == '\n' {
                    line += 1;
                    column = 1;
                } else {
                    column += 1;
                }
            }
        }
    }
    for (index, (entry, &(start, end))) in entries.iter().zip(&ranges).enumerate() {
        if index % 32 == 0 {
            yield_validation(cancel, deadline).await?;
        }
        for (offset, prefix) in [(start, "start"), (end, "end")] {
            let position = &positions[positions
                .binary_search_by_key(&offset, |position| position.offset)
                .unwrap()];
            if prefix == "start"
                && !source
                    .get(position.content_start..end)
                    .is_some_and(|text| text.starts_with(entry["signature"].as_str().unwrap()))
            {
                return Err(error("invalid declaration or source signature"));
            }
            if entry["range"][format!("{prefix}_line")] != serde_json::json!(position.line)
                || entry["range"][format!("{prefix}_column")] != serde_json::json!(position.column)
            {
                return Err(error("source positions disagree with byte range"));
            }
        }
    }
    check_invocation(cancel, deadline)
}
pub(super) fn error(message: &str) -> AgentError {
    AgentError::Tool(format!("experimental WASM: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bindings::huginn::rust_outline::snapshots::HostSnapshot;

    #[tokio::test]
    async fn host_validation_rejects_cancelled_or_expired_invocations_even_without_entries() {
        let output = r#"{"complete":true,"declarations":[],"diagnostics":[]}"#;
        let cancel = CancellationToken::new();
        let expired = tokio::time::Instant::now() - Duration::from_secs(1);
        let result = validate_output(output, "", &cancel, expired).await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exceeded deadline"));
        cancel.cancel();
        let future = tokio::time::Instant::now() + Duration::from_secs(DEADLINE_SECS);
        assert!(matches!(
            validate_output(output, "", &cancel, future).await,
            Err(AgentError::Cancelled)
        ));
    }

    #[tokio::test]
    async fn quota_violations_trap_instead_of_returning_an_ignorable_guest_error() {
        let activity = WasmActivity::default();
        activity.0.active.fetch_add(1, Ordering::AcqRel);
        let mut state = State {
            table: ResourceTable::new(),
            limits: wasmtime::StoreLimitsBuilder::new().build(),
            activity,
        };
        let resource = state
            .table
            .push(Snapshot {
                bytes: vec![1, 2],
                remaining: 2,
            })
            .unwrap();
        assert!(state
            .read(Resource::new_borrow(resource.rep()), 0, 3)
            .await
            .is_err());
    }
}
