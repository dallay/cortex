use huginn_application::permission_policy::*;

fn file_write_identity() -> WorkspaceGrantIdentity {
    WorkspaceGrantIdentity::new(
        "/workspace/project",
        Capability::FileWrite,
        "native.write_file",
        "/workspace/project/src/lib.rs",
    )
}

fn file_write_grant() -> WorkspaceGrant {
    WorkspaceGrant::explicit_user_choice(file_write_identity())
}

#[test]
fn managed_restrictions_override_user_profile_and_approval_mode() {
    let user = PolicySettings {
        profile: ProcessProfile::Workspace,
        approval_mode: ApprovalMode::OnRequest,
    };
    let managed = ManagedRestrictions {
        profile: Some(ProcessProfile::Restricted),
        approval_mode: Some(ApprovalMode::Always),
        denied_capabilities: Default::default(),
    };

    let policy = EffectivePolicy::resolve(user, Some(managed), ProjectTrust::default());

    assert_eq!(policy.profile(), ProcessProfile::Restricted);
    assert_eq!(policy.approval_mode(), ApprovalMode::Always);
    assert_eq!(policy.profile_source(), PolicySource::Managed);
    assert_eq!(policy.approval_source(), PolicySource::Managed);
}

#[test]
fn trust_requires_explicit_provenance_and_does_not_change_profile() {
    let unverified = ProjectTrust::default();
    assert_eq!(unverified.state(), TrustState::Untrusted);
    assert_eq!(unverified.provenance(), None);

    let policy = EffectivePolicy::resolve(
        PolicySettings::default(),
        None,
        ProjectTrust::explicit(TrustState::Trusted, TrustProvenance::UserConfiguration),
    );

    assert_eq!(policy.trust().state(), TrustState::Trusted);
    assert_eq!(
        policy.trust().provenance(),
        Some(TrustProvenance::UserConfiguration)
    );
    assert_eq!(policy.profile(), ProcessProfile::Restricted);
}

#[test]
fn on_request_allows_in_profile_work_and_escalates_only_bounded_capabilities() {
    let policy = EffectivePolicy::resolve(
        PolicySettings {
            profile: ProcessProfile::Workspace,
            approval_mode: ApprovalMode::OnRequest,
        },
        None,
        ProjectTrust::default(),
    )
    .with_escalations([Capability::Shell]);

    assert_eq!(
        policy.decide(Capability::FileRead, false),
        AuthorizationOutcome::Allowed
    );
    assert_eq!(
        policy.decide(Capability::Shell, true),
        AuthorizationOutcome::RequiresApproval
    );
    assert_eq!(
        policy.decide(Capability::Shell, false),
        AuthorizationOutcome::Denied
    );
    assert_eq!(
        policy.decide(Capability::Network, true),
        AuthorizationOutcome::Denied
    );
    assert_eq!(policy.profile(), ProcessProfile::Workspace);
}

#[test]
fn on_request_makes_out_of_profile_effects_prompt_eligible_without_optional_escalations() {
    let policy = EffectivePolicy::resolve(
        PolicySettings {
            profile: ProcessProfile::Restricted,
            approval_mode: ApprovalMode::OnRequest,
        },
        None,
        ProjectTrust::default(),
    );

    assert_eq!(
        policy.decide(Capability::FileWrite, true),
        AuthorizationOutcome::RequiresApproval
    );
    assert_eq!(
        policy.decide(Capability::FileWrite, false),
        AuthorizationOutcome::Denied
    );
}

#[test]
fn explicitly_empty_escalations_deny_out_of_profile_capabilities() {
    let policy = EffectivePolicy::resolve(PolicySettings::default(), None, ProjectTrust::default())
        .with_escalations([]);
    assert_eq!(
        policy.decide(Capability::Shell, true),
        AuthorizationOutcome::Denied
    );
    assert_eq!(
        policy.decide(Capability::FileRead, true),
        AuthorizationOutcome::Allowed
    );
}

#[test]
fn managed_denials_cannot_be_escalated() {
    let policy = EffectivePolicy::resolve(
        PolicySettings {
            profile: ProcessProfile::Workspace,
            approval_mode: ApprovalMode::OnRequest,
        },
        Some(ManagedRestrictions {
            denied_capabilities: [Capability::Shell].into(),
            ..ManagedRestrictions::default()
        }),
        ProjectTrust::default(),
    )
    .with_escalations([Capability::Shell]);

    assert_eq!(
        policy.decide(Capability::Shell, true),
        AuthorizationOutcome::Denied
    );
}

#[test]
fn user_denials_and_managed_denials_take_precedence_over_workspace_grants() {
    let user_rules = UserRules {
        denied_capabilities: [Capability::FileWrite].into(),
    };
    let user_denied = EffectivePolicy::resolve_with_rules(
        PolicySettings {
            profile: ProcessProfile::Restricted,
            approval_mode: ApprovalMode::OnRequest,
        },
        GlobalRules::default(),
        user_rules,
        None,
        ProjectTrust::default(),
    );

    assert_eq!(
        user_denied.decide_with_workspace_grant(
            &file_write_identity(),
            true,
            Some(&file_write_grant()),
        ),
        AuthorizationOutcome::Denied,
        "a workspace grant must not weaken a user prohibition"
    );
    assert_eq!(
        user_denied.denial_source(Capability::FileWrite),
        Some(PolicySource::User)
    );

    let managed_denied = EffectivePolicy::resolve_with_rules(
        PolicySettings {
            profile: ProcessProfile::FullAccess,
            approval_mode: ApprovalMode::OnRequest,
        },
        GlobalRules::default(),
        UserRules::default(),
        Some(ManagedRestrictions {
            denied_capabilities: [Capability::FileWrite].into(),
            ..ManagedRestrictions::default()
        }),
        ProjectTrust::default(),
    );
    assert_eq!(
        managed_denied.decide_with_workspace_grant(
            &file_write_identity(),
            true,
            Some(&file_write_grant()),
        ),
        AuthorizationOutcome::Denied,
        "managed denial must override both user rules and workspace grants"
    );
    assert_eq!(
        managed_denied.denial_source(Capability::FileWrite),
        Some(PolicySource::Managed)
    );
}

#[test]
fn workspace_grant_is_distinct_from_global_and_user_policy_rules() {
    let policy = EffectivePolicy::resolve_with_rules(
        PolicySettings {
            profile: ProcessProfile::Restricted,
            approval_mode: ApprovalMode::OnRequest,
        },
        GlobalRules::default(),
        UserRules::default(),
        None,
        ProjectTrust::default(),
    );

    assert_eq!(
        policy
            .decide_with_workspace_grant(&file_write_identity(), true, Some(&file_write_grant()),),
        AuthorizationOutcome::Allowed
    );
    assert_eq!(
        policy.decide_with_workspace_grant(&file_write_identity(), true, None),
        AuthorizationOutcome::RequiresApproval,
        "without a grant the unchanged OnRequest prompt remains eligible"
    );

    let global_denied = EffectivePolicy::resolve_with_rules(
        PolicySettings {
            profile: ProcessProfile::FullAccess,
            approval_mode: ApprovalMode::OnRequest,
        },
        GlobalRules {
            denied_capabilities: [Capability::FileWrite].into(),
        },
        UserRules::default(),
        None,
        ProjectTrust::default(),
    );
    assert_eq!(
        global_denied.decide_with_workspace_grant(
            &file_write_identity(),
            true,
            Some(&file_write_grant()),
        ),
        AuthorizationOutcome::Denied
    );
    assert_eq!(
        global_denied.denial_source(Capability::FileWrite),
        Some(PolicySource::Global)
    );
}

#[test]
fn workspace_grant_requires_an_exact_caller_supplied_identity() {
    let policy = EffectivePolicy::resolve(
        PolicySettings {
            profile: ProcessProfile::Restricted,
            approval_mode: ApprovalMode::OnRequest,
        },
        None,
        ProjectTrust::default(),
    );
    let grant = WorkspaceGrant::explicit_user_choice(WorkspaceGrantIdentity::new(
        "/workspace/project",
        Capability::FileWrite,
        "native.write_file",
        "/workspace/project/src/lib.rs",
    ));

    let exact = WorkspaceGrantIdentity::new(
        "/workspace/project",
        Capability::FileWrite,
        "native.write_file",
        "/workspace/project/src/lib.rs",
    );
    assert_eq!(
        policy.decide_with_workspace_grant(&exact, true, Some(&grant)),
        AuthorizationOutcome::Allowed
    );

    for mismatch in [
        WorkspaceGrantIdentity::new(
            "/workspace/other",
            Capability::FileWrite,
            "native.write_file",
            "/workspace/project/src/lib.rs",
        ),
        WorkspaceGrantIdentity::new(
            "/workspace/project",
            Capability::FileWrite,
            "native.edit_file",
            "/workspace/project/src/lib.rs",
        ),
        WorkspaceGrantIdentity::new(
            "/workspace/project",
            Capability::FileWrite,
            "native.write_file",
            "/workspace/project/src/",
        ),
        WorkspaceGrantIdentity::new(
            "/workspace/project",
            Capability::Shell,
            "native.write_file",
            "/workspace/project/src/lib.rs",
        ),
    ] {
        assert_eq!(
            policy.decide_with_workspace_grant(&mismatch, true, Some(&grant)),
            AuthorizationOutcome::RequiresApproval
        );
    }
}

#[test]
fn workspace_grant_does_not_bypass_always_approval() {
    let policy = EffectivePolicy::resolve(
        PolicySettings {
            profile: ProcessProfile::FullAccess,
            approval_mode: ApprovalMode::Always,
        },
        None,
        ProjectTrust::default(),
    );
    let identity = WorkspaceGrantIdentity::new(
        "/workspace/project",
        Capability::FileWrite,
        "native.write_file",
        "/workspace/project/src/lib.rs",
    );
    let grant = WorkspaceGrant::explicit_user_choice(identity.clone());

    assert_eq!(
        policy.decide_with_workspace_grant(&identity, true, Some(&grant)),
        AuthorizationOutcome::RequiresApproval
    );
    assert_eq!(
        policy.decide_with_workspace_grant(&identity, false, Some(&grant)),
        AuthorizationOutcome::Denied
    );
}

#[test]
fn managed_values_cannot_relax_user_restrictions() {
    let policy = EffectivePolicy::resolve(
        PolicySettings {
            profile: ProcessProfile::Restricted,
            approval_mode: ApprovalMode::Always,
        },
        Some(ManagedRestrictions {
            profile: Some(ProcessProfile::FullAccess),
            approval_mode: Some(ApprovalMode::OnRequest),
            denied_capabilities: Default::default(),
        }),
        ProjectTrust::default(),
    );

    assert_eq!(policy.profile(), ProcessProfile::Restricted);
    assert_eq!(policy.approval_mode(), ApprovalMode::Always);
}

#[test]
fn missing_approver_denies_escalation_and_always_mode_requires_approval() {
    let on_request = EffectivePolicy::resolve(
        PolicySettings {
            profile: ProcessProfile::Workspace,
            approval_mode: ApprovalMode::OnRequest,
        },
        None,
        ProjectTrust::default(),
    )
    .with_escalations([Capability::Shell]);
    assert_eq!(
        on_request.decide(Capability::Shell, false),
        AuthorizationOutcome::Denied
    );

    let always = EffectivePolicy::resolve(
        PolicySettings {
            profile: ProcessProfile::FullAccess,
            approval_mode: ApprovalMode::Always,
        },
        None,
        ProjectTrust::default(),
    );
    assert_eq!(
        always.decide(Capability::Shell, true),
        AuthorizationOutcome::RequiresApproval
    );
    assert_eq!(
        always.decide(Capability::Shell, false),
        AuthorizationOutcome::Denied
    );
}
