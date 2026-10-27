#[path = "support/hooks.rs"]
mod support;
use async_trait::async_trait;
use huginn_core::{
    hooks::{ToolHook, ToolHooks, ToolOutcome, ToolVerdict},
    AgentError, CancellationToken, Result, ToolCall, ToolContext,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use support::{compose, Author};

enum Behavior {
    Continue,
    Block(&'static str),
    Fail,
    Hang,
    Panic,
    Note(String),
}
struct Probe {
    behavior: Behavior,
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl ToolHook for Probe {
    async fn before(&self, _: &ToolCall, _: &ToolContext) -> Result<ToolVerdict> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.behavior {
            Behavior::Block(reason) => Ok(ToolVerdict::Block((*reason).into())),
            Behavior::Fail => Err(AgentError::Tool("injected".into())),
            Behavior::Hang => {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(ToolVerdict::Continue)
            }
            Behavior::Panic => panic!("hook panicked"),
            Behavior::Continue | Behavior::Note(_) => Ok(ToolVerdict::Continue),
        }
    }
    async fn after(
        &self,
        _: &ToolCall,
        _: ToolOutcome<'_>,
        _: &ToolContext,
    ) -> Result<Option<String>> {
        match &self.behavior {
            Behavior::Note(note) => Ok(Some(note.clone())),
            Behavior::Fail => Err(AgentError::Tool("injected".into())),
            _ => Ok(None),
        }
    }
}
fn probe(behavior: Behavior) -> (Arc<dyn ToolHook>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (
        Arc::new(Probe {
            behavior,
            calls: calls.clone(),
        }),
        calls,
    )
}
fn call() -> ToolCall {
    ToolCall {
        id: "call".into(),
        name: "shell".into(),
        arguments: serde_json::json!({"command": "rm -rf /"}),
    }
}
fn context() -> ToolContext {
    ToolContext {
        selected_skills: vec![],
        workspace: ".".into(),
        cancellation: CancellationToken::new(),
    }
}

#[tokio::test]
async fn hooks_run_in_id_order_and_the_first_block_wins() {
    let hooks = Arc::new(ToolHooks::default());
    let (allow, allow_calls) = probe(Behavior::Continue);
    let (block, block_calls) = probe(Behavior::Block("destructive command"));
    let (later, later_calls) = probe(Behavior::Block("never consulted"));
    let (mut supervisor, result) = compose(
        hooks.clone(),
        vec![Author {
            id: "guard",
            // Registration order differs from id order on purpose.
            hooks: vec![("c.later", later), ("b.block", block), ("a.allow", allow)],
            fail: false,
        }],
    )
    .await;
    result.unwrap();
    let blocked = hooks.before(&call(), &context()).await.unwrap();
    assert_eq!(blocked.hook, "b.block");
    assert_eq!(blocked.reason, "destructive command");
    assert_eq!(allow_calls.load(Ordering::SeqCst), 1);
    assert_eq!(block_calls.load(Ordering::SeqCst), 1);
    assert_eq!(later_calls.load(Ordering::SeqCst), 0);
    supervisor.shutdown().await;
}

#[tokio::test]
async fn failing_slow_or_panicking_hooks_fail_closed() {
    for behavior in [Behavior::Fail, Behavior::Hang, Behavior::Panic] {
        let hooks = Arc::new(ToolHooks::with_timeout(Duration::from_millis(50)));
        let (hook, _) = probe(behavior);
        let (mut supervisor, result) = compose(
            hooks.clone(),
            vec![Author {
                id: "guard",
                hooks: vec![("guard.faulty", hook)],
                fail: false,
            }],
        )
        .await;
        result.unwrap();
        let blocked = hooks.before(&call(), &context()).await.unwrap();
        assert_eq!(blocked.hook, "guard.faulty");
        assert!(blocked.reason.contains("hook failed"), "{}", blocked.reason);
        supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn notes_and_reasons_are_sanitized_and_bounded() {
    let hooks = Arc::new(ToolHooks::default());
    let (noisy, _) = probe(Behavior::Note(format!(
        "\u{1b}[31mred\u{202E}{}",
        "x".repeat(10_000)
    )));
    let (broken, _) = probe(Behavior::Fail);
    let (mut supervisor, result) = compose(
        hooks.clone(),
        vec![Author {
            id: "notes",
            hooks: vec![("a.noisy", noisy), ("b.broken", broken)],
            fail: false,
        }],
    )
    .await;
    result.unwrap();
    let outcome = ToolOutcome {
        output: "ok",
        is_error: false,
    };
    let notes = hooks.after(&call(), outcome, &context()).await;
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].hook, "a.noisy");
    assert!(notes[0].text.starts_with("[31mred"));
    assert!(!notes[0].text.contains('\u{1b}') && !notes[0].text.contains('\u{202E}'));
    assert!(notes[0].text.len() <= huginn_core::hooks::MAX_NOTE_BYTES);
    assert_eq!(notes[1].hook, "b.broken");
    assert!(notes[1].text.contains("hook failed"));
    supervisor.shutdown().await;
}

#[tokio::test]
async fn rolled_back_and_revoked_hooks_are_not_evaluated() {
    for fail in [false, true] {
        let hooks = Arc::new(ToolHooks::default());
        let (guard, calls) = probe(Behavior::Block("no"));
        let (mut supervisor, result) = compose(
            hooks.clone(),
            vec![Author {
                id: "guard",
                hooks: vec![("guard.shell", guard)],
                fail,
            }],
        )
        .await;
        if fail {
            assert!(result.is_err());
        } else {
            result.unwrap();
            assert!(hooks.before(&call(), &context()).await.is_some());
            supervisor.remove("guard").await.unwrap();
        }
        assert!(hooks.before(&call(), &context()).await.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(!fail));
        supervisor.shutdown().await;
    }
}
