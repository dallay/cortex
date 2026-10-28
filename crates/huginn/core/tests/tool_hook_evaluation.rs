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
    LongFail(String),
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
            Behavior::LongFail(text) => Err(AgentError::Tool(text.clone())),
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
            Behavior::LongFail(text) => Err(AgentError::Tool(text.clone())),
            Behavior::Hang => std::future::pending().await,
            Behavior::Panic => panic!("after hook panicked"),
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

async fn long_error_output(after: bool) -> String {
    let hooks = Arc::new(ToolHooks::default());
    let (hook, _) = probe(Behavior::LongFail("\u{1b}\u{202E}é".repeat(4096)));
    let (mut supervisor, result) = compose(
        hooks.clone(),
        vec![Author {
            id: "long-error",
            hooks: vec![("error.long", hook)],
            fail: false,
        }],
    )
    .await;
    result.unwrap();
    let text = if after {
        hooks
            .after(
                &call(),
                ToolOutcome {
                    output: "ok",
                    is_error: false,
                },
                &context(),
            )
            .await
            .remove(0)
            .text
    } else {
        hooks.before(&call(), &context()).await.unwrap().reason
    };
    supervisor.shutdown().await;
    text
}

fn assert_bounded_error(text: &str) {
    assert!(
        text.len() <= huginn_core::hooks::MAX_NOTE_BYTES,
        "{} bytes",
        text.len()
    );
    assert!(text.starts_with("hook failed: tool: "));
    assert!(text.contains('é'));
    assert!(!text.contains('\u{1b}') && !text.contains('\u{202E}'));
}

#[tokio::test]
async fn long_before_error_includes_prefix_within_utf8_budget() {
    assert_bounded_error(&long_error_output(false).await);
}

#[tokio::test]
async fn long_after_error_includes_prefix_within_utf8_budget() {
    assert_bounded_error(&long_error_output(true).await);
}

#[tokio::test]
async fn after_failures_are_visible_and_do_not_skip_later_hooks() {
    for (behavior, expected) in [
        (Behavior::Fail, "injected"),
        (Behavior::Hang, "timed out"),
        (Behavior::Panic, "panicked"),
    ] {
        let hooks = Arc::new(ToolHooks::with_timeout(Duration::from_millis(20)));
        let (failing, _) = probe(behavior);
        let (later, _) = probe(Behavior::Note("later observation".into()));
        let (mut supervisor, result) = compose(
            hooks.clone(),
            vec![Author {
                id: "after-errors",
                hooks: vec![("z.later", later), ("a.failing", failing)],
                fail: false,
            }],
        )
        .await;
        result.unwrap();
        let notes = hooks
            .after(
                &call(),
                ToolOutcome {
                    output: "failed effect",
                    is_error: true,
                },
                &context(),
            )
            .await;
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].hook, "a.failing");
        assert!(notes[0].text.starts_with("hook failed: "));
        assert!(notes[0].text.contains(expected), "{}", notes[0].text);
        assert_eq!(notes[1].hook, "z.later");
        assert_eq!(notes[1].text, "later observation");
        supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn sanitized_empty_notes_are_omitted_and_multibyte_notes_end_at_a_character_boundary() {
    let hooks = Arc::new(ToolHooks::default());
    let (empty, _) = probe(Behavior::Note("\u{0}\u{1b}\u{200e}\u{2069}".into()));
    let (unicode, _) = probe(Behavior::Note(format!("\t\n{}", "🦀".repeat(2048))));
    let (mut supervisor, result) = compose(
        hooks.clone(),
        vec![Author {
            id: "sanitize",
            hooks: vec![("a.empty", empty), ("b.unicode", unicode)],
            fail: false,
        }],
    )
    .await;
    result.unwrap();
    let notes = hooks
        .after(
            &call(),
            ToolOutcome {
                output: "ok",
                is_error: false,
            },
            &context(),
        )
        .await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].hook, "b.unicode");
    assert_eq!(
        notes[0].text,
        format!(
            "\t\n{}",
            "🦀".repeat((huginn_core::hooks::MAX_NOTE_BYTES - 2) / 4)
        )
    );
    supervisor.shutdown().await;
}

#[tokio::test]
async fn dropping_a_before_evaluation_cancels_the_hook_future() {
    struct PendingHook {
        started: Arc<tokio::sync::Notify>,
        dropped: Arc<AtomicUsize>,
    }
    struct OnDrop(Arc<AtomicUsize>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[async_trait]
    impl ToolHook for PendingHook {
        async fn before(&self, _: &ToolCall, _: &ToolContext) -> Result<ToolVerdict> {
            let _guard = OnDrop(self.dropped.clone());
            self.started.notify_one();
            std::future::pending().await
        }
    }
    let hooks = Arc::new(ToolHooks::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(AtomicUsize::new(0));
    let (mut supervisor, result) = compose(
        hooks.clone(),
        vec![Author {
            id: "pending",
            hooks: vec![(
                "pending.hook",
                Arc::new(PendingHook {
                    started: started.clone(),
                    dropped: dropped.clone(),
                }),
            )],
            fail: false,
        }],
    )
    .await;
    result.unwrap();
    let evaluation = tokio::spawn(async move { hooks.before(&call(), &context()).await });
    tokio::time::timeout(Duration::from_secs(1), started.notified())
        .await
        .unwrap();
    evaluation.abort();
    assert!(evaluation.await.unwrap_err().is_cancelled());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    supervisor.shutdown().await;
}
