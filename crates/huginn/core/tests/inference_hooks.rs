use async_trait::async_trait;
use huginn_core::{
    hooks::{
        inference_hooks_id, InferenceHook, InferenceHooks, InferenceHooksService, InferenceVerdict,
        MAX_NOTE_BYTES,
    },
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    AgentError, CancellationToken, Message, Result, Role, StepId, ToolContext, TurnId,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

struct Host(Arc<InferenceHooks>);
#[async_trait]
impl Plugin for Host {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "host.inference".into(),
            provides: vec![inference_hooks_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(inference_hooks_id(), InferenceHooksService(self.0.clone()))
    }
}

struct Author {
    hooks: Vec<(&'static str, Arc<dyn InferenceHook>)>,
    fail: bool,
    registry: Option<Arc<InferenceHooks>>,
    declare_dependency: bool,
}
#[async_trait]
impl Plugin for Author {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "author.inference".into(),
            provides: vec![],
            requires: if self.declare_dependency {
                vec![inference_hooks_id()]
            } else {
                vec![]
            },
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let hooks = match &self.registry {
            Some(registry) => registry.clone(),
            None => ctx
                .resolve::<InferenceHooksService>(&inference_hooks_id())?
                .0
                .clone(),
        };
        for (id, hook) in &self.hooks {
            hooks.register(ctx, (*id).into(), hook.clone())?;
            assert!(
                !hooks.ids().contains(&id.to_string()),
                "staged hooks must not run before commit"
            );
        }
        if self.fail {
            return Err(AgentError::Composition(
                "injected activation failure".into(),
            ));
        }
        Ok(())
    }
}
fn author(hooks: Vec<(&'static str, Arc<dyn InferenceHook>)>) -> Author {
    Author {
        hooks,
        fail: false,
        registry: None,
        declare_dependency: true,
    }
}
async fn compose(hooks: Arc<InferenceHooks>, author: Author) -> (Supervisor, Result<()>) {
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Host(hooks))).unwrap();
    supervisor.register(Box::new(author)).unwrap();
    let result = supervisor.start().await;
    (supervisor, result)
}
fn context() -> ToolContext {
    ToolContext {
        selected_skills: vec!["skill".into()],
        workspace: ".".into(),
        cancellation: CancellationToken::new(),
    }
}

enum Behavior {
    Continue,
    Block(String),
    Fail,
    Panic,
    Hang,
    Note(String),
}
struct Probe {
    behavior: Behavior,
    seen: Arc<Mutex<Vec<(TurnId, StepId)>>>,
}
#[async_trait]
impl InferenceHook for Probe {
    async fn before(
        &self,
        turn: TurnId,
        step: StepId,
        ctx: &ToolContext,
    ) -> Result<InferenceVerdict> {
        assert_eq!(ctx.selected_skills, ["skill"]);
        self.seen.lock().unwrap().push((turn, step));
        match &self.behavior {
            Behavior::Block(reason) => Ok(InferenceVerdict::Block(reason.clone())),
            Behavior::Fail => Err(AgentError::Tool("injected".into())),
            Behavior::Panic => panic!("injected panic"),
            Behavior::Hang => std::future::pending().await,
            Behavior::Continue | Behavior::Note(_) => Ok(InferenceVerdict::Continue),
        }
    }
    async fn after(
        &self,
        turn: TurnId,
        step: StepId,
        response: &Message,
        _: &ToolContext,
    ) -> Result<Option<String>> {
        assert_eq!(response.content, "model answer");
        self.seen.lock().unwrap().push((turn, step));
        match &self.behavior {
            Behavior::Fail => Err(AgentError::Tool("injected".into())),
            Behavior::Panic => panic!("injected panic"),
            Behavior::Hang => std::future::pending().await,
            Behavior::Note(note) => Ok(Some(note.clone())),
            Behavior::Continue | Behavior::Block(_) => Ok(None),
        }
    }
}
fn probe(behavior: Behavior) -> (Arc<dyn InferenceHook>, Arc<Mutex<Vec<(TurnId, StepId)>>>) {
    let seen = Arc::new(Mutex::new(vec![]));
    (
        Arc::new(Probe {
            behavior,
            seen: seen.clone(),
        }),
        seen,
    )
}

#[tokio::test]
async fn ordered_hooks_receive_identity_and_first_block_stops_evaluation() {
    let hooks = Arc::new(InferenceHooks::default());
    let (first, first_seen) = probe(Behavior::Continue);
    let (block, block_seen) = probe(Behavior::Block("no inference".into()));
    let (last, last_seen) = probe(Behavior::Block("too late".into()));
    let (mut supervisor, result) = compose(
        hooks.clone(),
        author(vec![
            ("z.last", last),
            ("b.block", block),
            ("a.first", first),
        ]),
    )
    .await;
    result.unwrap();
    assert_eq!(hooks.ids(), ["a.first", "b.block", "z.last"]);
    let blocked = hooks
        .before(TurnId(7), StepId(3), &context())
        .await
        .unwrap();
    assert_eq!(
        (blocked.hook.as_str(), blocked.reason.as_str()),
        ("b.block", "no inference")
    );
    assert_eq!(*first_seen.lock().unwrap(), [(TurnId(7), StepId(3))]);
    assert_eq!(*block_seen.lock().unwrap(), [(TurnId(7), StepId(3))]);
    assert!(last_seen.lock().unwrap().is_empty());
    supervisor.shutdown().await;
}

#[tokio::test]
async fn before_failures_close_the_gate_and_after_failures_do_not_hide_later_notes() {
    for (behavior, expected) in [
        (Behavior::Fail, "injected"),
        (Behavior::Panic, "panicked"),
        (Behavior::Hang, "timed out"),
    ] {
        let hooks = Arc::new(InferenceHooks::with_timeout(Duration::from_millis(20)));
        let (fault, _) = probe(behavior);
        let (later, seen) = probe(Behavior::Note("observed".into()));
        let (mut supervisor, result) = compose(
            hooks.clone(),
            author(vec![("z.later", later), ("a.fault", fault)]),
        )
        .await;
        result.unwrap();
        let blocked = hooks
            .before(TurnId(1), StepId(2), &context())
            .await
            .unwrap();
        assert_eq!(blocked.hook, "a.fault");
        assert!(blocked.reason.contains(expected), "{}", blocked.reason);
        assert!(seen.lock().unwrap().is_empty());
        let notes = hooks
            .after(
                TurnId(1),
                StepId(2),
                &Message::text(Role::Assistant, "model answer"),
                &context(),
            )
            .await;
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].hook, "a.fault");
        assert!(notes[0].text.contains(expected), "{}", notes[0].text);
        assert_eq!(
            (notes[1].hook.as_str(), notes[1].text.as_str()),
            ("z.later", "observed")
        );
        assert_eq!(*seen.lock().unwrap(), [(TurnId(1), StepId(2))]);
        supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn reasons_and_notes_are_sanitized_and_bounded_on_utf8_boundaries() {
    let hooks = Arc::new(InferenceHooks::default());
    let text = format!("\u{1b}\u{202e}{}", "🦀".repeat(MAX_NOTE_BYTES));
    let (block, _) = probe(Behavior::Block(text.clone()));
    let (note, _) = probe(Behavior::Note(text));
    let (mut supervisor, result) = compose(
        hooks.clone(),
        author(vec![("a.block", block), ("b.note", note)]),
    )
    .await;
    result.unwrap();
    let reason = hooks
        .before(TurnId(1), StepId(1), &context())
        .await
        .unwrap()
        .reason;
    let notes = hooks
        .after(
            TurnId(1),
            StepId(1),
            &Message::text(Role::Assistant, "model answer"),
            &context(),
        )
        .await;
    for value in [&reason, &notes[0].text] {
        assert_eq!(value.len(), MAX_NOTE_BYTES);
        assert_eq!(value, &"🦀".repeat(MAX_NOTE_BYTES / 4));
        assert!(!value.contains('\u{1b}') && !value.contains('\u{202e}'));
    }
    supervisor.shutdown().await;
}

#[tokio::test]
async fn registration_rolls_back_on_failure_and_revocation_allows_reuse() {
    let hooks = Arc::new(InferenceHooks::default());
    let (guard, seen) = probe(Behavior::Block("blocked".into()));
    let mut failed = author(vec![("guard", guard.clone())]);
    failed.fail = true;
    let (mut supervisor, result) = compose(hooks.clone(), failed).await;
    assert!(result.is_err());
    assert!(hooks.ids().is_empty());
    assert!(hooks
        .before(TurnId(1), StepId(1), &context())
        .await
        .is_none());
    assert!(seen.lock().unwrap().is_empty());
    supervisor.shutdown().await;

    let (mut supervisor, result) =
        compose(hooks.clone(), author(vec![("guard", guard.clone())])).await;
    result.unwrap();
    assert!(hooks
        .before(TurnId(1), StepId(1), &context())
        .await
        .is_some());
    supervisor.remove("author.inference").await.unwrap();
    assert!(hooks.ids().is_empty());
    assert!(hooks
        .before(TurnId(1), StepId(1), &context())
        .await
        .is_none());
    supervisor
        .register(Box::new(author(vec![("guard", guard)])))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(hooks
        .before(TurnId(2), StepId(1), &context())
        .await
        .is_some());
    assert_eq!(seen.lock().unwrap().len(), 2);
    supervisor.shutdown().await;
    assert!(hooks.ids().is_empty());
}

#[tokio::test]
async fn invalid_duplicate_and_unowned_registrations_are_rejected() {
    let hooks = Arc::new(InferenceHooks::default());
    let foreign = Arc::new(InferenceHooks::default());
    let (hook, _) = probe(Behavior::Continue);
    let mut collision = author(vec![("same", hook.clone()), ("same", hook.clone())]);
    let cases = [
        author(vec![("bad id", hook.clone())]),
        author(vec![("", hook.clone())]),
        {
            collision.fail = false;
            collision
        },
        Author {
            hooks: vec![("valid", hook.clone())],
            registry: Some(foreign.clone()),
            ..author(vec![])
        },
        Author {
            hooks: vec![("valid", hook)],
            registry: Some(hooks.clone()),
            declare_dependency: false,
            ..author(vec![])
        },
    ];
    for case in cases {
        let (mut supervisor, result) = compose(hooks.clone(), case).await;
        assert!(result.is_err());
        assert!(hooks.ids().is_empty());
        assert!(foreign.ids().is_empty());
        supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn cancelling_evaluation_drops_pending_before_and_after_hooks() {
    struct Pending {
        started: Arc<tokio::sync::Notify>,
        dropped: Arc<AtomicUsize>,
    }
    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Pending {
        async fn wait<T>(&self) -> T {
            let _guard = Guard(self.dropped.clone());
            self.started.notify_one();
            std::future::pending().await
        }
    }
    #[async_trait]
    impl InferenceHook for Pending {
        async fn before(&self, _: TurnId, _: StepId, _: &ToolContext) -> Result<InferenceVerdict> {
            self.wait().await
        }
        async fn after(
            &self,
            _: TurnId,
            _: StepId,
            _: &Message,
            _: &ToolContext,
        ) -> Result<Option<String>> {
            self.wait().await
        }
    }
    for after in [false, true] {
        let hooks = Arc::new(InferenceHooks::default());
        let started = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicUsize::new(0));
        let (mut supervisor, result) = compose(
            hooks.clone(),
            author(vec![(
                "pending",
                Arc::new(Pending {
                    started: started.clone(),
                    dropped: dropped.clone(),
                }),
            )]),
        )
        .await;
        result.unwrap();
        let evaluation = tokio::spawn(async move {
            if after {
                hooks
                    .after(
                        TurnId(1),
                        StepId(1),
                        &Message::text(Role::Assistant, "model answer"),
                        &context(),
                    )
                    .await;
            } else {
                hooks.before(TurnId(1), StepId(1), &context()).await;
            }
        });
        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .unwrap();
        evaluation.abort();
        assert!(evaluation.await.unwrap_err().is_cancelled());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        supervisor.shutdown().await;
    }
}
