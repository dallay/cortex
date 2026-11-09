use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessProfile {
    Restricted,
    Workspace,
    FullAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    OnRequest,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    FileRead,
    FileWrite,
    Shell,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustState {
    Untrusted,
    Trusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustProvenance {
    UserConfiguration,
    ManagedConfiguration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectTrust {
    state: TrustState,
    provenance: Option<TrustProvenance>,
}

impl Default for ProjectTrust {
    fn default() -> Self {
        Self {
            state: TrustState::Untrusted,
            provenance: None,
        }
    }
}

impl ProjectTrust {
    pub const fn explicit(state: TrustState, provenance: TrustProvenance) -> Self {
        Self {
            state,
            provenance: Some(provenance),
        }
    }

    pub const fn state(self) -> TrustState {
        self.state
    }

    pub const fn provenance(self) -> Option<TrustProvenance> {
        self.provenance
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicySource {
    Global,
    User,
    Managed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicySettings {
    pub profile: ProcessProfile,
    pub approval_mode: ApprovalMode,
}

impl Default for PolicySettings {
    fn default() -> Self {
        Self {
            profile: ProcessProfile::Restricted,
            approval_mode: ApprovalMode::OnRequest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ManagedRestrictions {
    pub profile: Option<ProcessProfile>,
    pub approval_mode: Option<ApprovalMode>,
    pub denied_capabilities: BTreeSet<Capability>,
}

/// Configurable global rules. These restrictions are independent from user rules
/// and workspace grants.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GlobalRules {
    pub denied_capabilities: BTreeSet<Capability>,
}

/// Configurable user rules. These rules can restrict policy, but workspace grants
/// are represented separately and cannot override a prohibition here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserRules {
    pub denied_capabilities: BTreeSet<Capability>,
}

/// Exact operation identity supplied by the caller after canonicalization.
/// This application contract performs no filesystem I/O or path resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGrantIdentity {
    workspace_root: String,
    capability: Capability,
    action: String,
    resource: String,
}

impl WorkspaceGrantIdentity {
    /// Constructs an identity from caller-verified canonical values.
    pub fn new(
        canonical_workspace_root: impl Into<String>,
        capability: Capability,
        action: impl Into<String>,
        canonical_resource: impl Into<String>,
    ) -> Self {
        Self {
            workspace_root: canonical_workspace_root.into(),
            capability,
            action: action.into(),
            resource: canonical_resource.into(),
        }
    }

    pub const fn capability(&self) -> Capability {
        self.capability
    }
}

/// An explicit grant bound to one exact caller-verified operation identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGrant {
    identity: WorkspaceGrantIdentity,
}

impl WorkspaceGrant {
    pub const fn explicit_user_choice(identity: WorkspaceGrantIdentity) -> Self {
        Self { identity }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationOutcome {
    Allowed,
    RequiresApproval,
    Denied,
}

/// Immutable authorization and isolation inputs resolved at composition time.
/// Trust is descriptive provenance only; it never changes profile authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectivePolicy {
    profile: ProcessProfile,
    approval_mode: ApprovalMode,
    trust: ProjectTrust,
    profile_source: PolicySource,
    approval_source: PolicySource,
    global_denied_capabilities: BTreeSet<Capability>,
    user_denied_capabilities: BTreeSet<Capability>,
    denied_capabilities: BTreeSet<Capability>,
    escalation_capabilities: BTreeSet<Capability>,
}

impl EffectivePolicy {
    pub fn resolve(
        user: PolicySettings,
        managed: Option<ManagedRestrictions>,
        trust: ProjectTrust,
    ) -> Self {
        Self::resolve_with_rules(
            user,
            GlobalRules::default(),
            UserRules::default(),
            managed,
            trust,
        )
    }

    pub fn resolve_with_rules(
        user: PolicySettings,
        global_rules: GlobalRules,
        user_rules: UserRules,
        managed: Option<ManagedRestrictions>,
        trust: ProjectTrust,
    ) -> Self {
        let managed_profile = managed
            .as_ref()
            .and_then(|restrictions| restrictions.profile);
        let (profile, profile_source) =
            managed_profile.map_or((user.profile, PolicySource::User), |managed_profile| {
                if managed_profile.restrictiveness() <= user.profile.restrictiveness() {
                    (managed_profile, PolicySource::Managed)
                } else {
                    (user.profile, PolicySource::User)
                }
            });
        let managed_approval = managed
            .as_ref()
            .and_then(|restrictions| restrictions.approval_mode);
        let (approval_mode, approval_source) = match managed_approval {
            Some(ApprovalMode::Always) => (ApprovalMode::Always, PolicySource::Managed),
            Some(ApprovalMode::OnRequest) if user.approval_mode == ApprovalMode::Always => {
                (ApprovalMode::Always, PolicySource::User)
            }
            Some(mode) => (mode, PolicySource::Managed),
            None => (user.approval_mode, PolicySource::User),
        };
        Self {
            profile,
            approval_mode,
            trust,
            profile_source,
            approval_source,
            global_denied_capabilities: global_rules.denied_capabilities,
            user_denied_capabilities: user_rules.denied_capabilities,
            denied_capabilities: managed
                .map(|restrictions| restrictions.denied_capabilities)
                .unwrap_or_default(),
            escalation_capabilities: BTreeSet::new(),
        }
    }

    /// Escalations are explicit, finite, and cannot override managed denials.
    pub fn with_escalations(mut self, capabilities: impl IntoIterator<Item = Capability>) -> Self {
        self.escalation_capabilities = capabilities.into_iter().collect();
        self
    }

    pub const fn profile(&self) -> ProcessProfile {
        self.profile
    }

    pub const fn approval_mode(&self) -> ApprovalMode {
        self.approval_mode
    }

    pub const fn trust(&self) -> ProjectTrust {
        self.trust
    }

    pub const fn profile_source(&self) -> PolicySource {
        self.profile_source
    }

    pub const fn approval_source(&self) -> PolicySource {
        self.approval_source
    }

    pub fn decide(&self, capability: Capability, approver_available: bool) -> AuthorizationOutcome {
        if self.is_denied(capability) {
            return AuthorizationOutcome::Denied;
        }
        if self.profile_allows(capability) {
            return match self.approval_mode {
                ApprovalMode::OnRequest => AuthorizationOutcome::Allowed,
                ApprovalMode::Always if approver_available => {
                    AuthorizationOutcome::RequiresApproval
                }
                ApprovalMode::Always => AuthorizationOutcome::Denied,
            };
        }
        if self.approval_mode == ApprovalMode::OnRequest
            && approver_available
            && (self.escalation_capabilities.is_empty()
                || self.escalation_capabilities.contains(&capability))
        {
            return AuthorizationOutcome::RequiresApproval;
        }
        AuthorizationOutcome::Denied
    }

    /// Resolve a previously explicit workspace grant after all policy prohibitions.
    /// This method grants no authority from behavioral instructions or metadata.
    pub fn decide_with_workspace_grant(
        &self,
        identity: &WorkspaceGrantIdentity,
        approver_available: bool,
        workspace_grant: Option<&WorkspaceGrant>,
    ) -> AuthorizationOutcome {
        let capability = identity.capability;
        if self.is_denied(capability) {
            return AuthorizationOutcome::Denied;
        }
        if workspace_grant.is_some_and(|grant| grant.identity == *identity) {
            return match self.approval_mode {
                ApprovalMode::OnRequest => AuthorizationOutcome::Allowed,
                ApprovalMode::Always => self.decide(capability, approver_available),
            };
        }
        self.decide(capability, approver_available)
    }

    pub fn denial_source(&self, capability: Capability) -> Option<PolicySource> {
        if self.denied_capabilities.contains(&capability) {
            Some(PolicySource::Managed)
        } else if self.global_denied_capabilities.contains(&capability) {
            Some(PolicySource::Global)
        } else if self.user_denied_capabilities.contains(&capability) {
            Some(PolicySource::User)
        } else {
            None
        }
    }

    fn is_denied(&self, capability: Capability) -> bool {
        self.denial_source(capability).is_some()
    }

    fn profile_allows(&self, capability: Capability) -> bool {
        match self.profile {
            ProcessProfile::Restricted => capability == Capability::FileRead,
            ProcessProfile::Workspace => {
                matches!(capability, Capability::FileRead | Capability::FileWrite)
            }
            ProcessProfile::FullAccess => true,
        }
    }
}

impl ProcessProfile {
    const fn restrictiveness(self) -> u8 {
        match self {
            Self::Restricted => 0,
            Self::Workspace => 1,
            Self::FullAccess => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn managed_denials_cannot_be_escalated() {
        let policy = EffectivePolicy::resolve(
            PolicySettings {
                profile: ProcessProfile::FullAccess,
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
            policy.decide_with_workspace_grant(
                &file_write_identity(),
                true,
                Some(&file_write_grant()),
            ),
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
}
