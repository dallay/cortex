use agent_core::{
    kernel::{Manifest, Plugin, PluginContext, ServiceId, State, Supervisor},
    AgentError, Result,
};
use async_trait::async_trait;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

struct Fixture {
    manifest: Manifest,
    log: Arc<Mutex<Vec<String>>>,
    fail: bool,
    cleaned: Arc<AtomicBool>,
}
#[async_trait]
impl Plugin for Fixture {
    fn manifest(&self) -> Manifest {
        self.manifest.clone()
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .push(format!("start {}", self.manifest.id));
        for id in &self.manifest.provides {
            ctx.provide(id.clone(), 42_u64)?;
        }
        let cancellation = ctx.cancellation.clone();
        let cleaned = self.cleaned.clone();
        ctx.own_task(tokio::spawn(async move {
            cancellation.cancelled().await;
            cleaned.store(true, Ordering::SeqCst);
        }));
        if self.fail {
            return Err(AgentError::Composition("fixture activation failure".into()));
        }
        Ok(())
    }
    async fn deactivate(&mut self) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .push(format!("stop {}", self.manifest.id));
        Ok(())
    }
}
fn plugin(
    id: &str,
    provides: Vec<ServiceId>,
    requires: Vec<ServiceId>,
    log: &Arc<Mutex<Vec<String>>>,
) -> Fixture {
    Fixture {
        manifest: Manifest {
            id: id.into(),
            provides,
            requires,
        },
        log: log.clone(),
        fail: false,
        cleaned: Arc::new(AtomicBool::new(false)),
    }
}

#[tokio::test]
async fn missing_provider_blocks_until_registered_then_tears_down_consumers_first() {
    let log = Arc::new(Mutex::new(vec![]));
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    let consumer = plugin("consumer", vec![], vec![service.clone()], &log);
    let cleaned = consumer.cleaned.clone();
    supervisor.register(Box::new(consumer)).unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(supervisor.diagnostics()[0].state, State::Blocked);
    supervisor
        .register(Box::new(plugin(
            "provider",
            vec![service.clone()],
            vec![],
            &log,
        )))
        .unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(*supervisor.resolve::<u64>(&service).unwrap(), 42);
    let generation = supervisor
        .diagnostics()
        .iter()
        .find(|d| d.manifest.id == "consumer")
        .unwrap()
        .generation;
    supervisor.remove("provider").await.unwrap();
    assert!(supervisor.resolve::<u64>(&service).is_err());
    assert!(cleaned.load(Ordering::SeqCst));
    assert_eq!(supervisor.diagnostics()[0].state, State::Blocked);
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            "start provider",
            "start consumer",
            "stop consumer",
            "stop provider"
        ]
    );
    supervisor
        .register(Box::new(plugin("provider", vec![service], vec![], &log)))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(supervisor.diagnostics()[0].generation > generation);
    supervisor.shutdown().await;
    assert!(supervisor
        .diagnostics()
        .iter()
        .all(|d| d.state == State::Unloaded));
}

#[tokio::test]
async fn failed_activation_rolls_back_services_and_owned_tasks() {
    let log = Arc::new(Mutex::new(vec![]));
    let service = ServiceId::new("test:value", 1);
    let mut fixture = plugin("failing", vec![service.clone()], vec![], &log);
    fixture.fail = true;
    let cleaned = fixture.cleaned.clone();
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(fixture)).unwrap();
    assert!(supervisor.start().await.is_err());
    assert!(supervisor.resolve::<u64>(&service).is_err());
    assert!(cleaned.load(Ordering::SeqCst));
    assert_eq!(supervisor.diagnostics()[0].state, State::Failed);
    assert!(supervisor.diagnostics()[0].last_error.is_some());
}

#[tokio::test]
async fn ambiguity_cycles_and_major_version_mismatch_are_explicit() {
    let log = Arc::new(Mutex::new(vec![]));
    let a = ServiceId::new("test:a", 1);
    let b = ServiceId::new("test:b", 1);
    let mut ambiguous = Supervisor::default();
    for id in ["a", "b"] {
        ambiguous
            .register(Box::new(plugin(id, vec![a.clone()], vec![], &log)))
            .unwrap();
    }
    assert!(ambiguous
        .start()
        .await
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    let mut cyclic = Supervisor::default();
    cyclic
        .register(Box::new(plugin(
            "a",
            vec![a.clone()],
            vec![b.clone()],
            &log,
        )))
        .unwrap();
    cyclic
        .register(Box::new(plugin("b", vec![b], vec![a.clone()], &log)))
        .unwrap();
    assert!(cyclic
        .start()
        .await
        .unwrap_err()
        .to_string()
        .contains("cycle"));
    let mut versions = Supervisor::default();
    versions
        .register(Box::new(plugin("provider", vec![a], vec![], &log)))
        .unwrap();
    versions
        .register(Box::new(plugin(
            "consumer",
            vec![],
            vec![ServiceId::new("test:a", 2)],
            &log,
        )))
        .unwrap();
    versions.start().await.unwrap();
    assert_eq!(versions.diagnostics()[0].state, State::Blocked);
    versions.shutdown().await;
}
