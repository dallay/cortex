use huginn_presentation::{event::OperationId, overlay::*};

#[cfg(feature = "test-driver")]
#[test]
fn exhausted_counters_close_fail_closed_and_never_reuse_input_lease() {
    let mut machine = OverlayMachine::default();
    let active = match machine.open(host_request(&machine, OverlayKind::Approval, "sender")) {
        OpenOutcome::Opened { active, .. } => active,
        _ => unreachable!(),
    };
    test_support::exhaust_counters(&mut machine);
    let ticket = machine.ticket();
    assert!(
        machine.close(active.id).is_some(),
        "exhaustion must retire permission authority"
    );
    assert!(!machine.accepts(ticket, FocusLease(u64::MAX)));
    assert!(matches!(
        machine.open(host_request(&machine, OverlayKind::Help, "new")),
        OpenOutcome::Rejected {
            reason: RejectReason::CounterExhausted,
            ..
        }
    ));
    assert!(machine.snapshot().is_none());
}

const fn host_request<T>(
    machine: &OverlayMachine<T>,
    kind: OverlayKind,
    payload: T,
) -> OpenRequest<T> {
    OpenRequest {
        kind,
        origin: Origin::Host,
        operation: OperationId(1),
        payload,
        ticket: machine.ticket(),
    }
}

#[test]
fn invalid_ticket_is_rejected_without_changing_focus_or_snapshot() {
    let mut machine = OverlayMachine::default();
    let first = match machine.open(host_request(&machine, OverlayKind::Help, "help")) {
        OpenOutcome::Opened { active, .. } => active,
        OpenOutcome::Rejected { .. } => panic!("first open should succeed"),
    };
    let stale = machine.ticket();
    let _second = match machine.open(host_request(&machine, OverlayKind::Settings, "settings")) {
        OpenOutcome::Opened { active, .. } => active,
        _ => panic!("second open should succeed"),
    };
    let before = machine.snapshot().unwrap();
    let result = machine.open(OpenRequest {
        kind: OverlayKind::Help,
        origin: Origin::Host,
        operation: OperationId(1),
        payload: "stale",
        ticket: stale,
    });
    assert!(matches!(
        result,
        OpenOutcome::Rejected {
            reason: RejectReason::StaleTicket,
            ..
        }
    ));
    assert_eq!(machine.snapshot().unwrap(), before);
    assert_ne!(before.id, first.id);
}

#[test]
fn valid_open_replaces_active_and_invalidates_old_ticket() {
    let mut machine = OverlayMachine::default();
    let first = match machine.open(host_request(&machine, OverlayKind::Help, "help")) {
        OpenOutcome::Opened { active, .. } => active,
        OpenOutcome::Rejected { .. } => panic!("first open should succeed"),
    };
    let stale = machine.ticket();
    let second = match machine.open(host_request(&machine, OverlayKind::Settings, "settings")) {
        OpenOutcome::Opened {
            retired: Some(old),
            active,
        } => {
            assert_eq!(old.id, first.id);
            active
        }
        _ => panic!("current host request should replace the ordinary overlay"),
    };
    let outcome = machine.open(OpenRequest {
        kind: OverlayKind::Help,
        origin: Origin::Host,
        operation: OperationId(1),
        payload: "stale",
        ticket: stale,
    });
    assert!(matches!(
        outcome,
        OpenOutcome::Rejected {
            reason: RejectReason::StaleTicket,
            ..
        }
    ));
    assert_eq!(machine.snapshot().unwrap().id, second.id);
}

#[test]
fn retired_payload_drop_can_reenter_after_caller_releases_lock() {
    use std::sync::{Arc, Mutex};
    struct Reenter(Arc<dyn Fn() + Send + Sync>);
    impl Drop for Reenter {
        fn drop(&mut self) {
            (self.0)();
        }
    }
    let machine = Arc::new(Mutex::new(OverlayMachine::<Reenter>::default()));
    let callback_machine = machine.clone();
    let payload = Reenter(Arc::new(move || {
        let machine = callback_machine.lock().unwrap();
        let _ = machine.snapshot();
    }));
    let ticket = machine.lock().unwrap().ticket();
    let active = {
        let mut locked = machine.lock().unwrap();
        match locked.open(OpenRequest {
            kind: OverlayKind::Help,
            origin: Origin::Host,
            operation: OperationId(1),
            payload,
            ticket,
        }) {
            OpenOutcome::Opened { active, .. } => active,
            OpenOutcome::Rejected { .. } => panic!("host overlay should open"),
        }
    };
    let retired = {
        let mut locked = machine.lock().unwrap();
        locked.close(active.id).unwrap()
    };
    drop(retired);
}

#[test]
fn revoked_origin_cannot_replace_active_overlay() {
    use async_trait::async_trait;
    use huginn_core::kernel::{Manifest, Plugin, PluginContext, Supervisor};
    use huginn_plugin_api::v1::{Contribution, Contributions, UiService};
    use std::sync::Arc;

    struct Host(Arc<Contributions>);
    #[async_trait]
    impl Plugin for Host {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "overlay-host".into(),
                provides: vec![huginn_plugin_api::v1::contributions_id()],
                requires: vec![],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> huginn_core::Result<()> {
            ctx.provide(
                huginn_plugin_api::v1::contributions_id(),
                UiService(self.0.clone()),
            )
        }
    }
    struct Ext;
    #[async_trait]
    impl Plugin for Ext {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "overlay-ext".into(),
                provides: vec![],
                requires: vec![huginn_plugin_api::v1::contributions_id()],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> huginn_core::Result<()> {
            let registry = ctx
                .resolve::<UiService>(&huginn_plugin_api::v1::contributions_id())?
                .0
                .clone();
            registry.register(
                ctx,
                "overlay-command".into(),
                Contribution::Command {
                    name: "/overlay".into(),
                    handler: Arc::new(|_| {
                        Ok(huginn_plugin_api::v1::CommandResult::Notice(
                            huginn_plugin_api::v1::View::text("overlay"),
                        ))
                    }),
                },
            )?;
            Ok(())
        }
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let registry = Arc::new(Contributions::default());
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(Host(registry.clone())))
            .unwrap();
        supervisor.register(Box::new(Ext)).unwrap();
        supervisor.start().await.unwrap();
        let command = registry.command("/overlay").unwrap();
        supervisor.remove("overlay-ext").await.unwrap();
        let mut machine = OverlayMachine::default();
        let active = match machine.open(host_request(&machine, OverlayKind::Settings, 1)) {
            OpenOutcome::Opened { active, .. } => active,
            _ => panic!("host open should succeed"),
        };
        let before = machine.snapshot().unwrap();
        let result = machine.open(OpenRequest {
            kind: OverlayKind::Help,
            origin: Origin::Command(command),
            operation: OperationId(2),
            payload: 2,
            ticket: machine.ticket(),
        });
        assert!(matches!(
            result,
            OpenOutcome::Rejected {
                reason: RejectReason::RevokedOrigin,
                ..
            }
        ));
        let after = machine.snapshot().unwrap();
        assert_eq!(after.id, active.id);
        assert_eq!(after.focus, before.focus);
        assert_eq!(after.revision, before.revision);
        drop(result);
    });
}
