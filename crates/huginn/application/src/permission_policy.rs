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
    escalation_capabilities: Option<BTreeSet<Capability>>,
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
            escalation_capabilities: None,
        }
    }

    /// Escalations are explicit, finite, and cannot override managed denials.
    pub fn with_escalations(mut self, capabilities: impl IntoIterator<Item = Capability>) -> Self {
        self.escalation_capabilities = Some(capabilities.into_iter().collect());
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
            && self
                .escalation_capabilities
                .as_ref()
                .is_none_or(|capabilities| capabilities.contains(&capability))
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
