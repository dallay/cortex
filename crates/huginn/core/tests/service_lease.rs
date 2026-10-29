use async_trait::async_trait;
use huginn_core::{
    kernel::{
        Manifest, Plugin, PluginContext, ServiceId, ServiceSlotHandle, SubstitutionPolicy,
        Supervisor,
    },
    AgentError, Result,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::Notify;

struct ValuePlugin {
    id: &'static str,
    service: ServiceId,
    value: u64,
    fail: bool,
}

#[async_trait]
impl Plugin for ValuePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: vec![self.service.clone()],
            requires: vec![],
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(self.service.clone(), self.value)?;
        if self.fail {
            return Err(huginn_core::AgentError::Composition(
                "injected activation failure".into(),
            ));
        }
        Ok(())
    }
}

struct DeactivationFailurePlugin(ServiceId);

#[async_trait]
impl Plugin for DeactivationFailurePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "original".into(),
            provides: vec![self.0.clone()],
            requires: Vec::new(),
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(self.0.clone(), 1_u64)
    }

    async fn deactivate(&mut self) -> Result<()> {
        Err(AgentError::Composition(
            "injected deactivation failure".into(),
        ))
    }
}

struct DeactivationTimeoutPlugin(ServiceId);

#[async_trait]
impl Plugin for DeactivationTimeoutPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "original-timeout".into(),
            provides: vec![self.0.clone()],
            requires: Vec::new(),
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(self.0.clone(), 1_u64)
    }

    async fn deactivate(&mut self) -> Result<()> {
        std::future::pending::<Result<()>>().await
    }
}

struct DependentPlugin {
    id: &'static str,
    requires: ServiceId,
}

#[async_trait]
impl Plugin for DependentPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: vec![],
            requires: vec![self.requires.clone()],
        }
    }

    async fn activate(&mut self, _ctx: &mut PluginContext) -> Result<()> {
        Ok(())
    }
}

struct RetainingSlotDependent {
    requires: ServiceId,
    handle: Arc<Mutex<Option<ServiceSlotHandle<u64>>>>,
}

#[async_trait]
impl Plugin for RetainingSlotDependent {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "slot-dependent".into(),
            provides: vec![],
            requires: vec![self.requires.clone()],
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        *self.handle.lock().unwrap() = Some(ctx.service_slot_handle(&self.requires)?);
        Ok(())
    }
}

struct InvalidSlotRequest {
    requires: ServiceId,
    requested: ServiceId,
    incompatible_type: bool,
}

#[async_trait]
impl Plugin for InvalidSlotRequest {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "invalid-slot-request".into(),
            provides: vec![],
            requires: vec![self.requires.clone()],
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        if self.incompatible_type {
            let _ = ctx.service_slot_handle::<String>(&self.requested)?;
        } else {
            let _ = ctx.service_slot_handle::<u64>(&self.requested)?;
        }
        Ok(())
    }
}

#[tokio::test]
async fn dependent_retains_context_slot_handle_across_provider_substitution() {
    let service = ServiceId::new("test:value", 1);
    let retained = Arc::new(Mutex::new(None));
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "original",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor
        .register(Box::new(RetainingSlotDependent {
            requires: service.clone(),
            handle: retained.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let handle = retained.lock().unwrap().take().unwrap();
    assert_eq!(*handle.acquire().unwrap(), 1);

    supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "replacement",
                service: service.clone(),
                value: 2,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

    assert_eq!(*handle.acquire().unwrap(), 2);

    supervisor.remove("replacement").await.unwrap();
    let error = handle.acquire().err().expect("removed slot is unavailable");
    assert!(error.to_string().contains("unavailable service slot"));
}

#[tokio::test]
async fn context_rejects_slot_handle_for_undeclared_service() {
    let required = ServiceId::new("test:required", 1);
    let undeclared = ServiceId::new("test:undeclared", 1);
    let mut supervisor = Supervisor::default();
    for (id, service) in [
        ("required-provider", &required),
        ("other-provider", &undeclared),
    ] {
        supervisor
            .register(Box::new(ValuePlugin {
                id,
                service: service.clone(),
                value: 1,
                fail: false,
            }))
            .unwrap();
    }
    supervisor
        .register(Box::new(InvalidSlotRequest {
            requires: required,
            requested: undeclared.clone(),
            incompatible_type: false,
        }))
        .unwrap();

    let error = supervisor.start().await.unwrap_err().to_string();

    assert!(
        error.contains("undeclared service slot"),
        "unexpected error: {error}"
    );
    assert!(error.contains(&undeclared.0));
}

#[tokio::test]
async fn context_rejects_slot_handle_with_incompatible_service_type() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "provider",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor
        .register(Box::new(InvalidSlotRequest {
            requires: service.clone(),
            requested: service.clone(),
            incompatible_type: true,
        }))
        .unwrap();

    let error = supervisor.start().await.unwrap_err().to_string();

    assert!(
        error.contains("incompatible service slot"),
        "unexpected error: {error}"
    );
    assert!(error.contains(&service.0));
}

#[tokio::test]
async fn ownership_and_dependents_are_observable_before_and_after_substitution() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "original",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor
        .register(Box::new(DependentPlugin {
            id: "dependent",
            requires: service.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    assert_eq!(
        supervisor.provider_of(&service).unwrap().plugin_id,
        "original"
    );
    assert_eq!(supervisor.dependents_of(&service), vec!["dependent"]);

    supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "replacement",
                service: service.clone(),
                value: 2,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

    assert_eq!(
        supervisor.provider_of(&service).unwrap().plugin_id,
        "replacement"
    );
    assert_eq!(supervisor.dependents_of(&service), vec!["dependent"]);
}

#[tokio::test]
async fn removing_provider_revokes_retained_service_handle() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "original",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let handle = supervisor.service_handle::<u64>(&service).unwrap();
    assert_eq!(*handle.acquire().unwrap(), 1);
    supervisor.remove("original").await.unwrap();
    assert!(handle.acquire().is_err());
}

#[tokio::test]
async fn timed_out_substitution_preserves_provider_until_existing_lease_drops() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "original",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let handle = supervisor.service_handle::<u64>(&service).unwrap();
    let lease = handle.acquire().unwrap();
    assert_eq!(*lease, 1);

    let timed_out = supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "replacement-timeout",
                service: service.clone(),
                value: 2,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_millis(1),
        )
        .await;
    assert!(timed_out.is_err());
    assert_eq!(*lease, 1);
    assert_eq!(*supervisor.resolve_lease::<u64>(&service).unwrap(), 1);
    let old_handle = supervisor.service_handle::<u64>(&service).unwrap();

    drop(lease);
    supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "replacement",
                service: service.clone(),
                value: 3,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(*supervisor.resolve_lease::<u64>(&service).unwrap(), 3);
    assert!(old_handle.acquire().is_err());
}

#[tokio::test]
async fn failed_alternative_activation_keeps_previous_provider_available() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "original",
            service: service.clone(),
            value: 7,
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    let old_handle = supervisor.service_handle::<u64>(&service).unwrap();

    let result = supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "failing-alternative",
                service: service.clone(),
                value: 9,
                fail: true,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await;

    assert!(result.is_err());
    assert_eq!(*old_handle.acquire().unwrap(), 7);
    assert_eq!(*supervisor.resolve_lease::<u64>(&service).unwrap(), 7);
    assert!(supervisor
        .substitution_diagnostics()
        .last()
        .unwrap()
        .outcome
        .is_err());
    let diagnostic = supervisor.substitution_diagnostics().last().unwrap();
    assert_eq!(diagnostic.service, service);
    assert_eq!(diagnostic.previous_owner, "original");
    assert_eq!(diagnostic.alternative_owner, "failing-alternative");
    assert!(diagnostic
        .outcome
        .as_ref()
        .unwrap_err()
        .contains("injected activation failure"));

    let retry = supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "failing-alternative",
                service: service.clone(),
                value: 11,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await;
    assert!(retry.is_ok());
    assert_eq!(*supervisor.resolve_lease::<u64>(&service).unwrap(), 11);
}

#[tokio::test]
async fn substitution_reports_retired_provider_deactivation_failure_after_commit() {
    let service = ServiceId::new("test:deactivation-failure", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(DeactivationFailurePlugin(service.clone())))
        .unwrap();
    supervisor.start().await.unwrap();

    let error = supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "replacement",
                service: service.clone(),
                value: 2,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("original: deactivation failed"));
    assert!(error.contains("injected deactivation failure"));
    assert_eq!(
        supervisor.provider_of(&service).unwrap().plugin_id,
        "replacement"
    );
    assert!(supervisor
        .substitution_diagnostics()
        .last()
        .unwrap()
        .outcome
        .as_ref()
        .unwrap_err()
        .contains("injected deactivation failure"));
}

#[tokio::test]
async fn retired_provider_deactivation_timeout_reports_committed_substitution() {
    let service = ServiceId::new("test:deactivation-timeout", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(DeactivationTimeoutPlugin(service.clone())))
        .unwrap();
    supervisor.start().await.unwrap();

    let error = supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "replacement-after-timeout",
                service: service.clone(),
                value: 2,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("substitution committed but finalization failed"));
    assert!(error.contains("original-timeout: deactivation timed out"));
    assert_eq!(
        supervisor.provider_of(&service).unwrap().plugin_id,
        "replacement-after-timeout"
    );
    let diagnostic = supervisor.substitution_diagnostics().last().unwrap();
    assert_eq!(diagnostic.alternative_owner, "replacement-after-timeout");
    assert!(diagnostic
        .outcome
        .as_ref()
        .unwrap_err()
        .contains("original-timeout: deactivation timed out"));
}

struct MultiServicePlugin {
    id: &'static str,
    services: Vec<(ServiceId, u64)>,
}

struct EventValuePlugin {
    id: &'static str,
    service: ServiceId,
    value: u64,
    fail: bool,
    events: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Plugin for EventValuePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: vec![self.service.clone()],
            requires: vec![],
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("activate:{}", self.id));
        ctx.provide(self.service.clone(), self.value)?;
        if self.fail {
            return Err(AgentError::Composition(
                "injected activation failure".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl Plugin for MultiServicePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: self.services.iter().map(|(id, _)| id.clone()).collect(),
            requires: vec![],
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        for (service, value) in &self.services {
            ctx.provide(service.clone(), *value)?;
        }
        Ok(())
    }
}

struct LifecycleOrderPlugin {
    id: &'static str,
    provides: Vec<(ServiceId, u64)>,
    requires: Vec<ServiceId>,
    events: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Plugin for LifecycleOrderPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: self
                .provides
                .iter()
                .map(|(service, _)| service.clone())
                .collect(),
            requires: self.requires.clone(),
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        for (service, value) in &self.provides {
            ctx.provide(service.clone(), *value)?;
        }
        Ok(())
    }

    async fn deactivate(&mut self) -> Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("deactivate:{}", self.id));
        Ok(())
    }
}

struct DrainDependentPlugin {
    id: &'static str,
    requires: Vec<ServiceId>,
    supports_drain: bool,
    fail_drain: bool,
    hang_drain: bool,
    hang_resume: bool,
    fail_resume: bool,
    resume_delay: Duration,
    events: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Plugin for DrainDependentPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: vec![],
            requires: self.requires.clone(),
        }
    }

    async fn activate(&mut self, _ctx: &mut PluginContext) -> Result<()> {
        Ok(())
    }

    fn supports_drain(&self) -> bool {
        self.supports_drain
    }

    async fn drain(&mut self) -> Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("drain:{}", self.id));
        if self.hang_drain {
            std::future::pending::<()>().await;
        }
        if self.fail_drain {
            return Err(AgentError::Composition(format!(
                "injected drain failure for {}",
                self.id
            )));
        }
        Ok(())
    }

    async fn resume(&mut self) -> Result<()> {
        if self.hang_resume {
            std::future::pending::<()>().await;
        }
        if !self.resume_delay.is_zero() {
            tokio::time::sleep(self.resume_delay).await;
        }
        if self.fail_resume {
            return Err(AgentError::Composition("injected resume failure".into()));
        }
        self.events
            .lock()
            .unwrap()
            .push(format!("resume:{}", self.id));
        Ok(())
    }
}

fn drain_dependent(
    id: &'static str,
    requires: Vec<ServiceId>,
    supports_drain: bool,
    fail_drain: bool,
    hang_drain: bool,
    events: &Arc<Mutex<Vec<String>>>,
) -> DrainDependentPlugin {
    DrainDependentPlugin {
        id,
        requires,
        supports_drain,
        fail_drain,
        hang_drain,
        hang_resume: false,
        fail_resume: false,
        resume_delay: Duration::ZERO,
        events: events.clone(),
    }
}

struct GateCloseObserver {
    requires: Vec<ServiceId>,
    closed: Arc<Notify>,
}

#[async_trait]
impl Plugin for GateCloseObserver {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "aaa.test.gate-close-observer".into(),
            provides: Vec::new(),
            requires: self.requires.clone(),
        }
    }

    async fn activate(&mut self, _: &mut PluginContext) -> Result<()> {
        Ok(())
    }

    fn supports_drain(&self) -> bool {
        true
    }

    async fn drain(&mut self) -> Result<()> {
        self.closed.notify_one();
        Ok(())
    }
}

#[tokio::test]
async fn substituting_multi_service_provider_preserves_its_services_and_adds_free_services() {
    let primary = ServiceId::new("test:primary", 1);
    let secondary = ServiceId::new("test:secondary", 1);
    let additional = ServiceId::new("test:additional", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(MultiServicePlugin {
            id: "original-multi",
            services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
        }))
        .unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    supervisor
        .register(Box::new(drain_dependent(
            "secondary-user",
            vec![secondary.clone()],
            true,
            false,
            false,
            &events,
        )))
        .unwrap();
    supervisor.start().await.unwrap();
    let old_secondary_handle = supervisor.service_handle::<u64>(&secondary).unwrap();

    let result = supervisor
        .substitute(
            &primary,
            Box::new(MultiServicePlugin {
                id: "replacement-multi",
                services: vec![
                    (primary.clone(), 10),
                    (secondary.clone(), 20),
                    (additional.clone(), 30),
                ],
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await;

    assert!(result.is_ok(), "substitution failed: {result:?}");
    assert_eq!(*supervisor.resolve_lease::<u64>(&primary).unwrap(), 10);
    assert_eq!(*supervisor.resolve_lease::<u64>(&secondary).unwrap(), 20);
    assert_eq!(*supervisor.resolve_lease::<u64>(&additional).unwrap(), 30);
    assert_eq!(
        supervisor.provider_of(&secondary).unwrap().plugin_id,
        "replacement-multi"
    );
    assert_eq!(
        supervisor.provider_of(&additional).unwrap().plugin_id,
        "replacement-multi"
    );
    assert_eq!(
        supervisor.provider_of(&primary).unwrap().generation,
        supervisor.provider_of(&secondary).unwrap().generation
    );
    assert!(old_secondary_handle.acquire().is_err());
    assert_eq!(
        *events.lock().unwrap(),
        ["drain:secondary-user", "resume:secondary-user"]
    );
    assert_eq!(
        supervisor
            .substitution_diagnostics()
            .last()
            .unwrap()
            .drained,
        ["secondary-user"]
    );
    assert!(supervisor
        .substitution_diagnostics()
        .last()
        .unwrap()
        .outcome
        .is_ok());
}

#[tokio::test]
async fn substitution_rejects_an_alternative_that_collides_on_an_additional_service() {
    let primary = ServiceId::new("test:primary", 1);
    let secondary = ServiceId::new("test:secondary", 1);
    let collision = ServiceId::new("test:owned", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(MultiServicePlugin {
            id: "original-multi",
            services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
        }))
        .unwrap();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "other-owner",
            service: collision.clone(),
            value: 3,
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let result = supervisor
        .substitute(
            &primary,
            Box::new(MultiServicePlugin {
                id: "replacement-multi",
                services: vec![
                    (primary.clone(), 10),
                    (secondary.clone(), 20),
                    (collision.clone(), 30),
                ],
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await;

    let error = result.unwrap_err().to_string();
    assert!(error.contains(&collision.0), "unexpected error: {error}");
    assert_eq!(*supervisor.resolve_lease::<u64>(&primary).unwrap(), 1);
    assert_eq!(*supervisor.resolve_lease::<u64>(&secondary).unwrap(), 2);
    assert_eq!(*supervisor.resolve_lease::<u64>(&collision).unwrap(), 3);
}

#[tokio::test]
async fn substitution_rejects_an_alternative_that_drops_an_existing_service() {
    let primary = ServiceId::new("test:primary", 1);
    let secondary = ServiceId::new("test:secondary", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(MultiServicePlugin {
            id: "original-multi",
            services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let result = supervisor
        .substitute(
            &primary,
            Box::new(MultiServicePlugin {
                id: "replacement-incomplete",
                services: vec![(primary.clone(), 10)],
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await;

    let error = result.unwrap_err().to_string();
    assert!(error.contains(&secondary.0), "unexpected error: {error}");
    assert_eq!(*supervisor.resolve_lease::<u64>(&primary).unwrap(), 1);
    assert_eq!(*supervisor.resolve_lease::<u64>(&secondary).unwrap(), 2);
    assert_eq!(
        supervisor.provider_of(&secondary).unwrap().plugin_id,
        "original-multi"
    );
}

#[test]
fn plugins_without_a_drain_override_do_not_claim_drain_support() {
    let plugin = DependentPlugin {
        id: "legacy-dependent",
        requires: ServiceId::new("test:value", 1),
    };

    assert!(!plugin.supports_drain());
}

#[tokio::test]
async fn alternative_activation_is_staged_before_dependents_drain() {
    let service = ServiceId::new("test:value", 1);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "original",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor
        .register(Box::new(drain_dependent(
            "dependent",
            vec![service.clone()],
            true,
            false,
            false,
            &events,
        )))
        .unwrap();
    supervisor.start().await.unwrap();

    supervisor
        .substitute(
            &service,
            Box::new(EventValuePlugin {
                id: "replacement",
                service: service.clone(),
                value: 2,
                fail: false,
                events: events.clone(),
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        [
            "activate:replacement",
            "drain:dependent",
            "resume:dependent"
        ]
    );
    assert_eq!(*supervisor.resolve_lease::<u64>(&service).unwrap(), 2);
}

#[tokio::test]
async fn require_full_drain_checks_dependents_of_every_service_from_the_provider() {
    let primary = ServiceId::new("test:primary", 1);
    let secondary = ServiceId::new("test:secondary", 1);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(MultiServicePlugin {
            id: "original-multi",
            services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
        }))
        .unwrap();
    supervisor
        .register(Box::new(drain_dependent(
            "secondary-user",
            vec![secondary.clone()],
            false,
            false,
            false,
            &events,
        )))
        .unwrap();
    supervisor.start().await.unwrap();

    let result = supervisor
        .substitute(
            &primary,
            Box::new(MultiServicePlugin {
                id: "replacement-multi",
                services: vec![(primary.clone(), 10), (secondary.clone(), 20)],
            }),
            SubstitutionPolicy::RequireFullDrain,
            Duration::from_secs(1),
        )
        .await;

    assert!(result.is_err());
    assert_eq!(*supervisor.resolve_lease::<u64>(&primary).unwrap(), 1);
    assert_eq!(*supervisor.resolve_lease::<u64>(&secondary).unwrap(), 2);
    assert!(events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn drain_failure_resumes_prior_dependents_reopens_leases_and_records_failure() {
    let primary = ServiceId::new("test:primary", 1);
    let secondary = ServiceId::new("test:secondary", 1);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(MultiServicePlugin {
            id: "original-multi",
            services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
        }))
        .unwrap();
    supervisor
        .register(Box::new(drain_dependent(
            "a-secondary-user",
            vec![secondary.clone()],
            true,
            false,
            false,
            &events,
        )))
        .unwrap();
    supervisor
        .register(Box::new(drain_dependent(
            "z-primary-user",
            vec![primary.clone()],
            true,
            true,
            false,
            &events,
        )))
        .unwrap();
    supervisor.start().await.unwrap();
    let old_handle = supervisor.service_handle::<u64>(&primary).unwrap();

    let result = supervisor
        .substitute(
            &primary,
            Box::new(MultiServicePlugin {
                id: "replacement-multi",
                services: vec![(primary.clone(), 10), (secondary.clone(), 20)],
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await;

    assert!(result.is_err());
    assert_eq!(*old_handle.acquire().unwrap(), 1);
    assert_eq!(*supervisor.resolve_lease::<u64>(&secondary).unwrap(), 2);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "drain:a-secondary-user",
            "drain:z-primary-user",
            "resume:z-primary-user",
            "resume:a-secondary-user"
        ]
    );
    let diagnostic = supervisor.substitution_diagnostics().last().unwrap();
    assert_eq!(diagnostic.service, primary);
    assert_eq!(diagnostic.previous_owner, "original-multi");
    assert_eq!(diagnostic.alternative_owner, "replacement-multi");
    assert!(diagnostic.outcome.is_err());
}

#[tokio::test]
async fn timed_out_drain_hook_is_bounded_and_reopens_the_previous_generation() {
    let service = ServiceId::new("test:value", 1);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "original",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor
        .register(Box::new(drain_dependent(
            "slow-dependent",
            vec![service.clone()],
            true,
            false,
            true,
            &events,
        )))
        .unwrap();
    supervisor.start().await.unwrap();
    let old_handle = supervisor.service_handle::<u64>(&service).unwrap();

    let bounded = tokio::time::timeout(
        Duration::from_millis(100),
        supervisor.substitute(
            &service,
            Box::new(ValuePlugin {
                id: "replacement",
                service: service.clone(),
                value: 2,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_millis(10),
        ),
    )
    .await;

    assert!(matches!(bounded, Ok(Err(_))));
    assert_eq!(*old_handle.acquire().unwrap(), 1);
    assert_eq!(
        *events.lock().unwrap(),
        ["drain:slow-dependent", "resume:slow-dependent"]
    );
    let diagnostic = supervisor.substitution_diagnostics().last().unwrap();
    assert!(diagnostic
        .outcome
        .as_ref()
        .unwrap_err()
        .contains("slow-dependent"));
    assert!(diagnostic.drained.is_empty());
}

#[tokio::test]
async fn report_undrained_lists_dependents_of_secondary_services() {
    let primary = ServiceId::new("test:primary", 1);
    let secondary = ServiceId::new("test:secondary", 1);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(MultiServicePlugin {
            id: "original-multi",
            services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
        }))
        .unwrap();
    supervisor
        .register(Box::new(drain_dependent(
            "secondary-user",
            vec![secondary.clone()],
            false,
            false,
            false,
            &events,
        )))
        .unwrap();
    supervisor.start().await.unwrap();

    supervisor
        .substitute(
            &primary,
            Box::new(MultiServicePlugin {
                id: "replacement-multi",
                services: vec![(primary.clone(), 10), (secondary.clone(), 20)],
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

    let diagnostic = supervisor.substitution_diagnostics().last().unwrap();
    assert_eq!(diagnostic.undrained, ["secondary-user"]);
    assert!(events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn removing_a_dependent_removes_it_from_dependent_introspection() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "provider",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor
        .register(Box::new(DependentPlugin {
            id: "dependent",
            requires: service.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    assert_eq!(supervisor.dependents_of(&service), ["dependent"]);

    supervisor.remove("dependent").await.unwrap();

    assert!(supervisor.dependents_of(&service).is_empty());
    assert_eq!(
        supervisor.provider_of(&service).unwrap().plugin_id,
        "provider"
    );
}

#[tokio::test]
async fn service_slot_tracks_replacement_while_existing_lease_keeps_previous_provider() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "original",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let slot = supervisor.service_slot_handle::<u64>(&service).unwrap();
    let previous = slot.acquire().unwrap();
    assert_eq!(*previous, 1);

    let substitution_service = service.clone();
    let mut substitution = tokio::spawn(async move {
        supervisor
            .substitute(
                &substitution_service,
                Box::new(ValuePlugin {
                    id: "replacement",
                    service: substitution_service.clone(),
                    value: 2,
                    fail: false,
                }),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        supervisor
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut substitution)
            .await
            .is_err()
    );

    assert_eq!(*previous, 1);
    drop(previous);
    let _supervisor = substitution.await.unwrap();
    assert_eq!(*slot.acquire().unwrap(), 2);
}

#[tokio::test]
async fn service_slot_acquisition_can_race_with_commit_and_multi_service_slots_refresh() {
    let primary = ServiceId::new("test:primary", 1);
    let secondary = ServiceId::new("test:secondary", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(MultiServicePlugin {
            id: "original-multi",
            services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let primary_slot = supervisor.service_slot_handle::<u64>(&primary).unwrap();
    let secondary_slot = supervisor.service_slot_handle::<u64>(&secondary).unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let commit_barrier = barrier.clone();
    let commit_primary = primary.clone();
    let commit_secondary = secondary.clone();
    let commit = tokio::spawn(async move {
        commit_barrier.wait().await;
        supervisor
            .substitute(
                &commit_primary,
                Box::new(MultiServicePlugin {
                    id: "replacement-multi",
                    services: vec![(commit_primary.clone(), 10), (commit_secondary.clone(), 20)],
                }),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        supervisor
    });

    barrier.wait().await;
    for _ in 0..100 {
        if let Ok(lease) = primary_slot.acquire() {
            assert!([1, 10].contains(&*lease));
        }
        tokio::task::yield_now().await;
    }

    let supervisor = commit.await.unwrap();
    assert_eq!(*primary_slot.acquire().unwrap(), 10);
    assert_eq!(*secondary_slot.acquire().unwrap(), 20);
    assert_eq!(
        supervisor.provider_of(&primary).unwrap().plugin_id,
        "replacement-multi"
    );
    assert_eq!(
        supervisor.provider_of(&secondary).unwrap().plugin_id,
        "replacement-multi"
    );
}

#[tokio::test]
async fn timed_out_slot_substitution_keeps_active_lease_and_reopens_admission() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "original",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let slot = supervisor.service_slot_handle::<u64>(&service).unwrap();
    let in_flight = slot.acquire().unwrap();
    let result = supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "timed-out-replacement",
                service: service.clone(),
                value: 2,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_millis(1),
        )
        .await;

    assert!(result.is_err());
    assert_eq!(*in_flight, 1);
    assert_eq!(*slot.acquire().unwrap(), 1);
    drop(in_flight);

    supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "replacement",
                service: service.clone(),
                value: 3,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(*slot.acquire().unwrap(), 3);
}

#[tokio::test]
async fn multi_service_slots_reject_new_leases_while_old_generation_drains() {
    let primary = ServiceId::new("test:primary", 1);
    let secondary = ServiceId::new("test:secondary", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(MultiServicePlugin {
            id: "original-multi",
            services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
        }))
        .unwrap();
    let gates_closed = Arc::new(Notify::new());
    supervisor
        .register(Box::new(GateCloseObserver {
            requires: vec![primary.clone(), secondary.clone()],
            closed: gates_closed.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let primary_slot = supervisor.service_slot_handle::<u64>(&primary).unwrap();
    let secondary_slot = supervisor.service_slot_handle::<u64>(&secondary).unwrap();
    let old_lease = primary_slot.acquire().unwrap();
    assert_eq!(*old_lease, 1);
    assert_eq!(*secondary_slot.acquire().unwrap(), 2);

    let substitution_primary = primary.clone();
    let substitution_secondary = secondary.clone();
    let substitution = tokio::spawn(async move {
        let result = supervisor
            .substitute(
                &substitution_primary,
                Box::new(MultiServicePlugin {
                    id: "replacement-multi",
                    services: vec![
                        (substitution_primary.clone(), 10),
                        (substitution_secondary.clone(), 20),
                    ],
                }),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(2),
            )
            .await;
        (supervisor, result)
    });

    tokio::time::timeout(Duration::from_secs(1), gates_closed.notified())
        .await
        .expect("substitution did not close the old gates");
    assert!(!substitution.is_finished());
    assert!(primary_slot.acquire().is_err());
    assert!(secondary_slot.acquire().is_err());
    assert_eq!(*old_lease, 1);

    drop(old_lease);
    let (mut supervisor, result) = substitution.await.unwrap();
    result.unwrap();
    assert_eq!(*primary_slot.acquire().unwrap(), 10);
    assert_eq!(*secondary_slot.acquire().unwrap(), 20);
    assert_eq!(
        supervisor.provider_of(&primary).unwrap().plugin_id,
        "replacement-multi"
    );
    assert_eq!(
        supervisor.provider_of(&secondary).unwrap().plugin_id,
        "replacement-multi"
    );
    supervisor.shutdown().await;
}

#[tokio::test]
async fn concurrent_multi_slot_lease_pairs_never_report_mixed_values() {
    let primary = ServiceId::new("test:primary", 1);
    let secondary = ServiceId::new("test:secondary", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(MultiServicePlugin {
            id: "original-multi",
            services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
        }))
        .unwrap();
    let gates_closed = Arc::new(Notify::new());
    supervisor
        .register(Box::new(GateCloseObserver {
            requires: vec![primary.clone(), secondary.clone()],
            closed: gates_closed.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let primary_slot = supervisor.service_slot_handle::<u64>(&primary).unwrap();
    let secondary_slot = supervisor.service_slot_handle::<u64>(&secondary).unwrap();
    let old_primary = primary_slot.acquire().unwrap();
    let old_secondary = secondary_slot.acquire().unwrap();
    assert_eq!((*old_primary, *old_secondary), (1, 2));
    drop(old_primary);
    drop(old_secondary);

    // Keep an old-generation lease so the substitution remains in its closed-gate
    // phase while the concurrent acquisition task probes both IDs.
    let held = primary_slot.acquire().unwrap();
    let substitution_primary = primary.clone();
    let substitution_secondary = secondary.clone();
    let substitution = tokio::spawn(async move {
        let result = supervisor
            .substitute(
                &substitution_primary,
                Box::new(MultiServicePlugin {
                    id: "replacement-multi",
                    services: vec![
                        (substitution_primary.clone(), 10),
                        (substitution_secondary.clone(), 20),
                    ],
                }),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(2),
            )
            .await;
        (supervisor, result)
    });
    tokio::time::timeout(Duration::from_secs(1), gates_closed.notified())
        .await
        .expect("substitution did not close the old gates");

    let stop = Arc::new(AtomicBool::new(false));
    let attempted = Arc::new(Notify::new());
    let worker_primary = primary_slot.clone();
    let worker_secondary = secondary_slot.clone();
    let worker_stop = stop.clone();
    let worker_attempted = attempted.clone();
    let worker = tokio::spawn(async move {
        let mut signalled = false;
        let mut observed = Vec::new();
        loop {
            let first = worker_primary.acquire();
            let second = worker_secondary.acquire();
            if let (Ok(first), Ok(second)) = (first, second) {
                let pair = (*first, *second);
                assert!(
                    pair == (1, 2) || pair == (10, 20),
                    "mixed slot pair: {pair:?}"
                );
                observed.push(pair);
            } else if !signalled {
                signalled = true;
                worker_attempted.notify_one();
            }
            if worker_stop.load(Ordering::Acquire) {
                break;
            }
            tokio::task::yield_now().await;
        }
        observed
    });

    tokio::time::timeout(Duration::from_secs(1), attempted.notified())
        .await
        .expect("acquisition worker did not attempt both handles");
    drop(held);
    let (mut supervisor, result) = substitution.await.unwrap();
    result.unwrap();
    stop.store(true, Ordering::Release);
    let observed = worker.await.unwrap();
    assert!(observed.contains(&(10, 20)));
    assert!(observed
        .iter()
        .all(|pair| *pair == (1, 2) || *pair == (10, 20)));
    supervisor.shutdown().await;
}

#[tokio::test]
async fn resolved_arc_remains_on_old_value_after_provider_substitution() {
    let service = ServiceId::new("test:value", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ValuePlugin {
            id: "original",
            service: service.clone(),
            value: 1,
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    let retained = supervisor.resolve::<u64>(&service).unwrap();
    assert_eq!(*retained, 1);

    supervisor
        .substitute(
            &service,
            Box::new(ValuePlugin {
                id: "replacement",
                service: service.clone(),
                value: 2,
                fail: false,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

    assert_eq!(*retained, 1);
    assert_eq!(*supervisor.resolve::<u64>(&service).unwrap(), 2);
    supervisor.shutdown().await;
}

#[tokio::test]
async fn dropping_supervisor_revokes_fixed_and_dynamic_handles_but_preserves_active_leases() {
    let primary = ServiceId::new("test:primary", 1);
    let secondary = ServiceId::new("test:secondary", 1);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(MultiServicePlugin {
            id: "original",
            services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    let fixed = supervisor.service_handle::<u64>(&primary).unwrap();
    let primary_slot = supervisor.service_slot_handle::<u64>(&primary).unwrap();
    let secondary_slot = supervisor.service_slot_handle::<u64>(&secondary).unwrap();
    let active = primary_slot.acquire().unwrap();

    drop(supervisor);

    assert!(fixed.acquire().is_err());
    assert!(primary_slot.acquire().is_err());
    assert!(secondary_slot.acquire().is_err());
    assert_eq!(*active, 1);
}

#[tokio::test]
async fn cancelled_drain_and_idle_wait_reopen_all_gates_and_resume_attempted_dependents() {
    for hang_drain in [true, false] {
        let primary = ServiceId::new("test:primary", 1);
        let secondary = ServiceId::new("test:secondary", 1);
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(MultiServicePlugin {
                id: "original",
                services: vec![(primary.clone(), 1), (secondary.clone(), 2)],
            }))
            .unwrap();
        for id in ["a-user", "z-user"] {
            let mut plugin = drain_dependent(
                id,
                vec![secondary.clone()],
                true,
                false,
                hang_drain && id == "z-user",
                &events,
            );
            // Exercise resume that yields, so synchronous Drop cannot complete rollback.
            plugin.resume_delay = Duration::from_millis(10);
            supervisor.register(Box::new(plugin)).unwrap();
        }
        supervisor.start().await.unwrap();
        let primary_slot = supervisor.service_slot_handle::<u64>(&primary).unwrap();
        let secondary_slot = supervisor.service_slot_handle::<u64>(&secondary).unwrap();
        let active = primary_slot.acquire().unwrap();
        let mut substitution = Box::pin(supervisor.substitute(
            &primary,
            Box::new(MultiServicePlugin {
                id: "replacement",
                services: vec![(primary.clone(), 10), (secondary.clone(), 20)],
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(60),
        ));
        assert!(futures::poll!(substitution.as_mut()).is_pending());
        assert!(primary_slot.acquire().is_err());
        assert!(secondary_slot.acquire().is_err());

        drop(substitution);

        assert_eq!(*primary_slot.acquire().unwrap(), 1);
        assert_eq!(*secondary_slot.acquire().unwrap(), 2);
        assert_eq!(*active, 1);
        let diagnostic = supervisor.substitution_diagnostics().last().unwrap();
        assert!(diagnostic
            .outcome
            .as_ref()
            .unwrap_err()
            .contains("cancelled"));
        assert_eq!(diagnostic.drained.len(), if hang_drain { 1 } else { 2 });
        // Recovery must progress without another lifecycle call to drive it.
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|event| event == "resume:a-user")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        supervisor.start().await.unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            [
                "drain:a-user",
                "drain:z-user",
                "resume:z-user",
                "resume:a-user"
            ]
        );
        assert!(!supervisor
            .diagnostics()
            .iter()
            .any(|d| d.manifest.id == "replacement"));
        assert_eq!(
            supervisor.provider_of(&primary).unwrap().plugin_id,
            "original"
        );
        supervisor.remove("z-user").await.unwrap();
        drop(active);
        supervisor
            .substitute(
                &primary,
                Box::new(MultiServicePlugin {
                    id: "replacement",
                    services: vec![(primary.clone(), 10), (secondary.clone(), 20)],
                }),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(*secondary_slot.acquire().unwrap(), 20);
    }
}

#[tokio::test]
async fn resume_attempts_continue_after_timeout_and_error_on_commit_and_rollback() {
    for fail_drain in [false, true] {
        let service = ServiceId::new("test:value", 1);
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(ValuePlugin {
                id: "original",
                service: service.clone(),
                value: 1,
                fail: false,
            }))
            .unwrap();
        for id in ["a-healthy", "b-error", "m-slow", "z-timeout"] {
            let mut plugin = drain_dependent(
                id,
                vec![service.clone()],
                true,
                fail_drain && id == "z-timeout",
                false,
                &events,
            );
            plugin.hang_resume = id == "z-timeout";
            plugin.fail_resume = id == "b-error";
            if id == "m-slow" {
                plugin.resume_delay = Duration::from_millis(10);
            }
            supervisor.register(Box::new(plugin)).unwrap();
        }
        supervisor.start().await.unwrap();
        let error = supervisor
            .substitute(
                &service,
                Box::new(ValuePlugin {
                    id: "replacement",
                    service: service.clone(),
                    value: 2,
                    fail: false,
                }),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(1),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("z-timeout: resume timed out"), "{error}");
        assert!(
            error.contains("b-error: composition: injected resume failure"),
            "{error}"
        );
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event == "resume:m-slow"));
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event == "resume:a-healthy"));
        assert_eq!(
            *supervisor.resolve_lease::<u64>(&service).unwrap(),
            if fail_drain { 1 } else { 2 }
        );
    }
}

#[tokio::test]
async fn shutdown_stops_dependents_before_the_replacement_provider() {
    let service = ServiceId::new("test:shutdown-order", 1);
    let later_dependency = ServiceId::new("test:later-dependency", 1);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(LifecycleOrderPlugin {
            id: "a-original",
            provides: vec![(service.clone(), 1)],
            requires: Vec::new(),
            events: events.clone(),
        }))
        .unwrap();
    supervisor
        .register(Box::new(LifecycleOrderPlugin {
            id: "b-later-dependency",
            provides: vec![(later_dependency.clone(), 3)],
            requires: Vec::new(),
            events: events.clone(),
        }))
        .unwrap();
    supervisor
        .register(Box::new(LifecycleOrderPlugin {
            id: "c-dependent",
            provides: Vec::new(),
            requires: vec![service.clone()],
            events: events.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    supervisor
        .substitute(
            &service,
            Box::new(LifecycleOrderPlugin {
                id: "replacement",
                provides: vec![(service.clone(), 2)],
                requires: vec![later_dependency],
                events: events.clone(),
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    events.lock().unwrap().clear();

    supervisor.shutdown().await;

    assert_eq!(
        *events.lock().unwrap(),
        [
            "deactivate:c-dependent",
            "deactivate:replacement",
            "deactivate:b-later-dependency"
        ]
    );
}

#[tokio::test]
async fn substitution_rejects_replacement_that_depends_on_its_own_dependent() {
    let service = ServiceId::new("test:cycle-target", 1);
    let dependent_service = ServiceId::new("test:dependent-service", 1);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(LifecycleOrderPlugin {
            id: "a-original",
            provides: vec![(service.clone(), 1)],
            requires: Vec::new(),
            events: events.clone(),
        }))
        .unwrap();
    supervisor
        .register(Box::new(LifecycleOrderPlugin {
            id: "b-dependent",
            provides: vec![(dependent_service.clone(), 3)],
            requires: vec![service.clone()],
            events: events.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let error = supervisor
        .substitute(
            &service,
            Box::new(LifecycleOrderPlugin {
                id: "replacement",
                provides: vec![(service.clone(), 2)],
                requires: vec![dependent_service.clone()],
                events,
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("cannot be ordered"), "{error}");
    assert_eq!(
        supervisor.provider_of(&service).unwrap().plugin_id,
        "a-original"
    );
    assert_eq!(
        supervisor
            .provider_of(&dependent_service)
            .unwrap()
            .plugin_id,
        "b-dependent"
    );
}
