#![cfg(debug_assertions)]

use async_trait::async_trait;
use huginn_core::kernel::{
    test_support, Manifest, Plugin, PluginContext, ServiceId, SubstitutionPolicy, Supervisor,
};
use huginn_core::Result;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

struct DropSignal(Arc<AtomicUsize>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct ResourcePlugin {
    id: &'static str,
    service: ServiceId,
    value: u64,
    dropped: Option<Arc<AtomicUsize>>,
}

#[async_trait]
impl Plugin for ResourcePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: vec![self.service.clone()],
            requires: vec![],
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(self.service.clone(), self.value)?;
        if let Some(dropped) = &self.dropped {
            ctx.own_resource(DropSignal(dropped.clone()));
        }
        Ok(())
    }
}

fn provider(service: &ServiceId) -> ResourcePlugin {
    ResourcePlugin {
        id: "original",
        service: service.clone(),
        value: 1,
        dropped: None,
    }
}

fn alternative(service: &ServiceId, dropped: Arc<AtomicUsize>) -> ResourcePlugin {
    ResourcePlugin {
        id: "alternative",
        service: service.clone(),
        value: 2,
        dropped: Some(dropped),
    }
}

struct CommitCallbackPlugin {
    service: ServiceId,
    published: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

#[async_trait]
impl Plugin for CommitCallbackPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "commit-callback".into(),
            provides: vec![self.service.clone()],
            requires: vec![],
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(self.service.clone(), 42_u64)?;
        ctx.own_resource(DropSignal(self.dropped.clone()));
        let published = self.published.clone();
        ctx.on_commit(move || {
            published.fetch_add(1, Ordering::SeqCst);
        });
        Ok(())
    }
}

struct RequiringAlternative {
    service: ServiceId,
    requires: Vec<ServiceId>,
    activated: Arc<AtomicBool>,
    dropped: Arc<AtomicUsize>,
    committed: Arc<AtomicBool>,
}

#[async_trait]
impl Plugin for RequiringAlternative {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "requiring-alternative".into(),
            provides: vec![self.service.clone()],
            requires: self.requires.clone(),
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        self.activated.store(true, Ordering::SeqCst);
        ctx.own_resource(DropSignal(self.dropped.clone()));
        ctx.provide(self.service.clone(), 2_u64)?;
        let committed = self.committed.clone();
        ctx.on_commit(move || committed.store(true, Ordering::SeqCst));
        Ok(())
    }
}

#[tokio::test]
async fn staging_validates_requirements_and_cleans_up_invalid_alternatives() {
    for requirement in ["missing", "self", "available"] {
        let service = ServiceId::new("test:value", 1);
        let dependency = ServiceId::new("test:dependency", 1);
        let mut supervisor = Supervisor::default();
        supervisor.register(Box::new(provider(&service))).unwrap();
        if requirement == "available" {
            supervisor
                .register(Box::new(ResourcePlugin {
                    id: "dependency",
                    service: dependency.clone(),
                    value: 10,
                    dropped: None,
                }))
                .unwrap();
        }
        supervisor.start().await.unwrap();
        let activated = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicUsize::new(0));
        let committed = Arc::new(AtomicBool::new(false));
        let result = supervisor
            .substitute(
                &service,
                Box::new(RequiringAlternative {
                    service: service.clone(),
                    requires: vec![if requirement == "self" {
                        service.clone()
                    } else {
                        dependency.clone()
                    }],
                    activated: activated.clone(),
                    dropped: dropped.clone(),
                    committed: committed.clone(),
                }),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(1),
            )
            .await;
        if requirement == "available" {
            result.unwrap();
            assert!(activated.load(Ordering::SeqCst));
            assert!(committed.load(Ordering::SeqCst));
            assert_eq!(*supervisor.resolve_lease::<u64>(&service).unwrap(), 2);
            assert_eq!(
                supervisor.dependents_of(&dependency),
                ["requiring-alternative"]
            );
        } else {
            let error = result.unwrap_err().to_string();
            if requirement == "missing" {
                assert!(error.contains(&dependency.0), "{error}");
                assert!(!activated.load(Ordering::SeqCst));
                assert_eq!(dropped.load(Ordering::SeqCst), 0);
            } else {
                assert!(error.contains("requires a service it provides"), "{error}");
                assert!(activated.load(Ordering::SeqCst));
                assert_eq!(dropped.load(Ordering::SeqCst), 1);
            }
            assert!(!committed.load(Ordering::SeqCst));
            assert!(supervisor
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.manifest.id != "requiring-alternative"));
            assert_eq!(*supervisor.resolve_lease::<u64>(&service).unwrap(), 1);
            assert_eq!(
                supervisor.provider_of(&service).unwrap().plugin_id,
                "original"
            );
        }
    }
}

#[tokio::test]
async fn lease_counter_overflow_returns_an_error_without_poisoning_the_gate() {
    let service = ServiceId::new("test:overflow", 1);
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(provider(&service))).unwrap();
    supervisor.start().await.unwrap();
    let slot = supervisor.service_slot_handle::<u64>(&service).unwrap();
    test_support::force_lease_counter_overflow(&supervisor, &service).unwrap();

    assert!(slot.acquire().is_err());
    assert!(test_support::lease_locks_are_unpoisoned(
        &supervisor,
        &service
    ));
}

#[tokio::test]
async fn missing_gate_after_staging_discards_alternative_and_records_abort() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(provider(&service))).unwrap();
    supervisor.start().await.unwrap();
    assert!(test_support::remove_lease_gate(&mut supervisor, &service));
    let dropped = Arc::new(AtomicUsize::new(0));

    let result = supervisor
        .substitute(
            &service,
            Box::new(alternative(&service, dropped.clone())),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await;

    assert!(result.is_err());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(supervisor
        .diagnostics()
        .iter()
        .all(|diagnostic| diagnostic.manifest.id != "alternative"));
    assert_eq!(
        supervisor.provider_of(&service).unwrap().plugin_id,
        "original"
    );
    let diagnostic = supervisor.substitution_diagnostics().last().unwrap();
    assert_eq!(diagnostic.alternative_owner, "alternative");
    assert!(diagnostic.outcome.is_err());
}

#[tokio::test]
async fn gate_close_failure_discards_staged_alternative_and_preserves_owner() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(provider(&service))).unwrap();
    supervisor.start().await.unwrap();
    test_support::poison_lease_gate(&supervisor, &service).unwrap();
    let dropped = Arc::new(AtomicUsize::new(0));

    let result = supervisor
        .substitute(
            &service,
            Box::new(alternative(&service, dropped.clone())),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await;

    assert!(result.is_err());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(supervisor
        .diagnostics()
        .iter()
        .all(|diagnostic| diagnostic.manifest.id != "alternative"));
    assert_eq!(
        supervisor.provider_of(&service).unwrap().plugin_id,
        "original"
    );
    let diagnostic = supervisor.substitution_diagnostics().last().unwrap();
    assert_eq!(diagnostic.alternative_owner, "alternative");
    assert!(diagnostic.outcome.is_err());
}

#[tokio::test]
async fn failed_slot_publication_does_not_run_commit_callbacks() {
    let service = ServiceId::new("test:commit-callback", 1);
    let mut supervisor = Supervisor::default();
    let published = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    supervisor
        .register(Box::new(CommitCallbackPlugin {
            service: service.clone(),
            published: published.clone(),
            dropped: dropped.clone(),
        }))
        .unwrap();
    test_support::poison_service_slots(&supervisor);

    let result = supervisor.start().await;

    assert!(result.is_err());
    assert_eq!(published.load(Ordering::SeqCst), 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(supervisor.resolve::<u64>(&service).is_err());
}
