use rook_core::{AuthType, ConnectionConfig, ProviderKind, QuotaWindowThresholds, ValidationError};
use shared_kernel::ProviderId;

pub(super) fn validate_base_fields(
    provider_runtime_id: &ProviderId,
    name: &str,
    priority: u8,
    config: &ConnectionConfig,
) -> Result<(), ValidationError> {
    if provider_runtime_id.as_str().trim().is_empty() {
        return Err(ValidationError::EmptyRuntimeId);
    }
    if name.trim().is_empty() {
        return Err(ValidationError::EmptyName);
    }
    if name.chars().count() > 256 {
        return Err(ValidationError::NameTooLong);
    }
    if !(1..=255).contains(&priority) {
        return Err(ValidationError::PriorityOutOfRange);
    }
    validate_config(config)
}

pub(super) fn validate_config(config: &ConnectionConfig) -> Result<(), ValidationError> {
    if config.max_concurrent < 1 {
        return Err(ValidationError::MaxConcurrentTooLow);
    }
    let QuotaWindowThresholds { warning, error } = config.quota_window_thresholds;
    if !warning.is_finite()
        || !error.is_finite()
        || !(0.0..=1.0).contains(&warning)
        || !(0.0..=1.0).contains(&error)
    {
        return Err(ValidationError::QuotaThresholdOutOfRange);
    }
    if error <= warning {
        return Err(ValidationError::QuotaThresholdOrder);
    }
    Ok(())
}

pub(super) fn validate_provider_config(
    provider_kind: ProviderKind,
    config: &ConnectionConfig,
    endpoint_policy: &rook_core::EndpointPolicy,
    is_active: bool,
) -> Result<(), ValidationError> {
    if provider_kind == ProviderKind::OpenAiCompatible {
        let Some(base_url) = config.base_url.as_deref() else {
            return Err(ValidationError::InvalidBaseUrl);
        };
        let endpoint =
            reqwest::Url::parse(base_url).map_err(|_| ValidationError::InvalidBaseUrl)?;
        if is_active {
            endpoint_policy
                .validate_url(&endpoint)
                .map_err(|_| ValidationError::InvalidBaseUrl)?;
        } else if endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || !matches!(endpoint.scheme(), "https" | "http")
            || endpoint.host_str().is_some_and(is_cloud_metadata_host)
        {
            return Err(ValidationError::InvalidBaseUrl);
        }
        if config.supported_models.is_empty()
            || config
                .supported_models
                .iter()
                .any(|m| m.as_str().trim().is_empty())
        {
            return Err(ValidationError::EmptySupportedModels);
        }
        if config.api_format.unwrap_or(rook_core::ApiFormat::OpenAI) != rook_core::ApiFormat::OpenAI
        {
            return Err(ValidationError::UnsupportedApiFormat);
        }
        return Ok(());
    }
    if let Some(base_url) = config.base_url.as_deref() {
        if reqwest::Url::parse(base_url)
            .ok()
            .and_then(|url| url.host_str().map(is_cloud_metadata_host))
            .unwrap_or(false)
        {
            return Err(ValidationError::CloudMetadataEndpointNotAllowed);
        }
    }

    if provider_kind != ProviderKind::AzureOpenAI {
        return Ok(());
    }
    let has_deployment = config
        .default_model
        .as_ref()
        .is_some_and(|model| !model.as_str().trim().is_empty());
    if !has_deployment {
        return Err(ValidationError::AzureOpenAIRequiresDeployment);
    }
    let Some(base_url) = config.base_url.as_deref() else {
        return Err(ValidationError::AzureOpenAIRequiresEndpoint);
    };
    let Ok(endpoint) = reqwest::Url::parse(base_url) else {
        return Err(ValidationError::AzureOpenAIRequiresEndpoint);
    };
    let host = endpoint.host_str().unwrap_or_default();
    let azure_host = host
        .strip_suffix(".openai.azure.com")
        .is_some_and(|resource| !resource.is_empty())
        || host
            .strip_suffix(".services.ai.azure.com")
            .is_some_and(|resource| !resource.is_empty());
    if endpoint.scheme() != "https"
        || !azure_host
        || endpoint.username() != ""
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !matches!(endpoint.path(), "" | "/")
    {
        return Err(ValidationError::AzureOpenAIRequiresEndpoint);
    }
    Ok(())
}

/// Block cloud instance-metadata destinations without blocking private
/// self-hosted providers (for example local Ollama or an internal gateway).
pub(super) fn is_cloud_metadata_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(&host);
    if matches!(
        host,
        "metadata.google.internal"
            | "metadata.google"
            | "metadata"
            | "instance-data.ec2.internal"
            | "metadata.azure.com"
            | "metadata.tencentyun.com"
            | "metadata.oraclecloud.com"
    ) {
        return true;
    }

    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => is_cloud_metadata_ipv4(ip),
        Ok(std::net::IpAddr::V6(ip)) => ip
            .to_ipv4()
            .map(is_cloud_metadata_ipv4)
            .unwrap_or_else(|| ip.to_string().eq_ignore_ascii_case("fd00:ec2::254")),
        Err(_) => false,
    }
}

fn is_cloud_metadata_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    (octets[0] == 169 && octets[1] == 254) || ip == std::net::Ipv4Addr::new(100, 100, 100, 200)
}

pub(super) fn normalize_api_format(provider_kind: ProviderKind, config: &mut ConnectionConfig) {
    if config.api_format.is_none() {
        config.api_format = Some(match provider_kind {
            ProviderKind::Anthropic => rook_core::ApiFormat::Anthropic,
            _ => rook_core::ApiFormat::OpenAI,
        });
    }
}

pub(super) fn validate_provider_auth_type(
    provider_kind: ProviderKind,
    auth_type: AuthType,
) -> Result<(), ValidationError> {
    if provider_kind == ProviderKind::AzureOpenAI && auth_type != AuthType::OAuth {
        return Err(ValidationError::AzureOpenAIRequiresOAuth);
    }
    if provider_kind == ProviderKind::OpenAiCompatible && auth_type != AuthType::ApiKey {
        return Err(ValidationError::AuthTypeCredentialMismatch);
    }
    Ok(())
}
