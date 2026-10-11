use async_trait::async_trait;
use huginn_core::{
    kernel::{Activation, Manifest, Plugin, PluginContext, Supervisor},
    AgentError, Result,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

struct Guard(Arc<AtomicUsize>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct Probe {
    drops: Arc<AtomicUsize>,
    lifetime: Arc<Mutex<Option<Activation>>>,
    fail: bool,
}
#[async_trait]
impl Plugin for Probe {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "probe".into(),
            provides: vec![],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let lifetime = ctx.activation();
        assert!(
            !lifetime.is_active(),
            "activation effects must remain staged"
        );
        *self.lifetime.lock().unwrap() = Some(lifetime);
        ctx.own(Guard(self.drops.clone()));
        if self.fail {
            Err(AgentError::Composition("injected".into()))
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn owned_registrations_publish_on_success_and_drop_on_every_exit() {
    for exit in ["failure", "remove", "shutdown", "drop"] {
        let drops = Arc::new(AtomicUsize::new(0));
        let lifetime = Arc::new(Mutex::new(None));
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(Probe {
                drops: drops.clone(),
                lifetime: lifetime.clone(),
                fail: exit == "failure",
            }))
            .unwrap();
        let result = supervisor.start().await;
        assert_eq!(result.is_err(), exit == "failure");
        let token = lifetime.lock().unwrap().clone().unwrap();
        assert_eq!(token.is_active(), exit != "failure");
        match exit {
            "remove" => supervisor.remove("probe").await.unwrap(),
            "shutdown" => supervisor.shutdown().await,
            _ => {}
        }
        drop(supervisor);
        assert!(!token.is_active());
        assert_eq!(drops.load(Ordering::SeqCst), 1, "{exit}");
    }
}
