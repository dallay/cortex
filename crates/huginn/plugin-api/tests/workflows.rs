use async_trait::async_trait;
use huginn_core::CancellationToken;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    v1::*,
    Result,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Host(Arc<Contributions>);
#[async_trait]
impl Plugin for Host {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "host".into(),
            provides: vec![contributions_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(contributions_id(), UiService(self.0.clone()))
    }
}
struct Author {
    calls: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    fail: bool,
}
#[async_trait]
impl Plugin for Author {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "author".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        let calls = self.calls.clone();
        let entered = self.entered.clone();
        ui.register(
            ctx,
            "author.action".into(),
            Contribution::Command {
                name: "/action".into(),
                handler: Arc::new(move |_| {
                    let calls = calls.clone();
                    let entered = entered.clone();
                    Ok(CommandResult::Workflow(Workflow::new(
                        "Action",
                        "Wait",
                        vec![],
                        Arc::new(move |_, _| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            let entered = entered.clone();
                            Box::pin(async move {
                                entered.notify_one();
                                std::future::pending::<()>().await;
                                Ok(CommandResult::Notice(View::text("late")))
                            })
                        }),
                    )?))
                }),
            },
        )?;
        if self.fail {
            return Err(huginn_core::AgentError::Composition("injected".into()));
        }
        Ok(())
    }
}
#[tokio::test]
async fn declarative_action_does_not_execute_in_callback_and_old_generation_cannot_execute_replacement(
) {
    let ui = Arc::new(Contributions::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Host(ui.clone()))).unwrap();
    supervisor
        .register(Box::new(Author {
            calls: calls.clone(),
            entered: entered.clone(),
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    let CommandResult::Workflow(workflow) = ui.command("/action").unwrap().run("").unwrap() else {
        panic!()
    };
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let action = workflow.clone();
    let pending = tokio::spawn(async move {
        action
            .execute(FormInput::default(), CancellationToken::new())
            .await
    });
    entered.notified().await;
    supervisor.remove("author").await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    supervisor
        .register(Box::new(Author {
            calls: calls.clone(),
            entered,
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(workflow
        .execute(FormInput::default(), CancellationToken::new())
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(ui.command("/action").unwrap().is_active());
}

#[tokio::test]
async fn failed_activation_rolls_back_async_contribution_without_executing_it() {
    let ui = Arc::new(Contributions::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Host(ui.clone()))).unwrap();
    supervisor.start().await.unwrap();
    supervisor
        .register(Box::new(Author {
            calls: calls.clone(),
            entered: Arc::default(),
            fail: true,
        }))
        .unwrap();
    assert!(supervisor.start().await.is_err());
    assert!(ui.command("/action").is_none());
    assert!(ui.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn secret_defaults_and_duplicate_fields_are_rejected() {
    let handler: WorkflowHandler =
        Arc::new(|_, _| Box::pin(async { Ok(CommandResult::Notice(View::text("ok"))) }));
    let secret = Field {
        id: "key".into(),
        label: "API key".into(),
        kind: FieldKind::Secret,
        initial: "sentinel".into(),
    };
    assert!(Workflow::new("Form", "", vec![secret.clone()], handler.clone()).is_err());
    let public = Field {
        initial: String::new(),
        kind: FieldKind::Text,
        ..secret
    };
    assert!(Workflow::new("Form", "", vec![public.clone(), public], handler).is_err());
}
