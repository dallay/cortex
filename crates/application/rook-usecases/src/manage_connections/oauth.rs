use super::*;

impl ManageConnections {
    pub async fn refresh_expiring_oauth_tokens(&self) -> usize {
        self.oauth_token_manager.refresh_expiring().await
    }

    pub fn start_oauth_refresh_task(&self) {
        let manager = self.oauth_token_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(crate::oauth_tokens::refresh_interval());
            interval.tick().await;
            loop {
                interval.tick().await;
                let refreshed = manager.refresh_expiring().await;
                if refreshed > 0 {
                    tracing::info!(refreshed, "refreshed expiring provider OAuth tokens");
                }
            }
        });
    }

    pub async fn start_oauth_flow(
        &self,
        provider_kind: ProviderKind,
        request: StartOAuthFlowRequest,
    ) -> ManageConnectionsResult<OAuthFlowStart> {
        self.prune_oauth_flows().await;
        if !matches!(
            provider_kind,
            ProviderKind::AzureOpenAI | ProviderKind::Gemini
        ) {
            return Err(ManageConnectionsError::OAuth(
                "OAuth is not supported for this provider".to_string(),
            ));
        }
        let existing_connection = match request.connection_id {
            Some(id) => {
                let connection = self.get(&id).await?.ok_or(RepositoryError::NotFound(id))?;
                if connection.provider_kind != provider_kind
                    || connection.auth_type != AuthType::OAuth
                {
                    return Err(ManageConnectionsError::OAuth(
                        "connection does not match provider OAuth type".to_string(),
                    ));
                }
                Some(connection)
            }
            None => None,
        };
        let existing_oauth = existing_connection.as_ref().and_then(|connection| {
            if let Credentials::OAuth {
                oauth_client_id,
                oauth_client_secret,
                oauth_tenant_id,
                project_id,
                ..
            } = &connection.credentials
            {
                Some((
                    oauth_client_id,
                    oauth_client_secret,
                    oauth_tenant_id,
                    project_id,
                ))
            } else {
                None
            }
        });
        let provided_client_id = request.client_id.trim().to_string();
        let client_id = if provided_client_id.is_empty() {
            existing_oauth
                .map(|(client_id, ..)| client_id.clone())
                .unwrap_or_default()
        } else {
            provided_client_id
        };
        if client_id.trim().is_empty() {
            return Err(ManageConnectionsError::OAuth(
                "clientId is required".to_string(),
            ));
        }
        validate_base_fields(
            &request.provider_runtime_id,
            &request.name,
            request.priority,
            &request.config,
        )?;
        validate_provider_config(
            provider_kind,
            &request.config,
            &rook_core::EndpointPolicy::default(),
            false,
        )?;
        let project_id = match request
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|project| !project.is_empty())
        {
            Some(project_id) => project_id.to_string(),
            None => match existing_oauth {
                Some((_, _, _, project_id)) => self.key_manager.decrypt(project_id.as_str())?,
                None => String::new(),
            },
        };
        if provider_kind == ProviderKind::Gemini && project_id.trim().is_empty() {
            return Err(ManageConnectionsError::OAuth(
                "projectId is required for Gemini OAuth".to_string(),
            ));
        }
        if request.flow_type == OAuthFlowType::AuthorizationCode {
            validate_oauth_redirect_uri(&request.redirect_uri)?;
        }
        if request.flow_type == OAuthFlowType::DeviceCode
            && provider_kind != ProviderKind::AzureOpenAI
        {
            return Err(ManageConnectionsError::OAuth(
                "Device Code is only supported for Azure OpenAI".to_string(),
            ));
        }
        if self.oauth_flows.len() >= 50 {
            return Err(ManageConnectionsError::OAuth(
                "too many pending OAuth flows".to_string(),
            ));
        }

        let tenant_id = request
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|tenant| !tenant.is_empty())
            .map(str::to_string)
            .or_else(|| existing_oauth.and_then(|(_, _, tenant_id, _)| tenant_id.clone()));
        let client_secret = match request
            .client_secret
            .filter(|secret| !secret.trim().is_empty())
        {
            Some(secret) => Some(secret),
            None => existing_oauth
                .and_then(|(_, secret, _, _)| secret.as_ref())
                .map(|secret| self.key_manager.decrypt(secret.as_str()))
                .transpose()?,
        };
        let scopes = oauth_scopes(provider_kind);
        let flow_id = ConnectionId::new().to_string();
        let state = Uuid::new_v4().simple().to_string();
        let code_verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let code_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
        let pending_connection = PendingOAuthConnection {
            provider_kind,
            provider_runtime_id: request.provider_runtime_id,
            name: request.name,
            priority: request.priority,
            is_active: request.is_active,
            config: request.config,
            client_id,
            client_secret: (request.flow_type == OAuthFlowType::AuthorizationCode)
                .then_some(client_secret)
                .flatten(),
            tenant_id,
            redirect_uri: request.redirect_uri,
            project_id,
            scopes,
            existing_connection_id: request.connection_id,
        };
        let mut pending = PendingOAuthFlow {
            flow_id: flow_id.clone(),
            state: state.clone(),
            code_verifier,
            connection: pending_connection,
            flow_type: request.flow_type,
            status: "pending".to_string(),
            message: None,
            expires_at: Utc::now().timestamp() + 600,
            device_code: None,
            poll_interval_secs: 5,
            next_poll_at: 0,
            connection_id: None,
        };

        let (
            authorization_url,
            user_code,
            verification_uri,
            message,
            interval_secs,
            expires_in_secs,
        ) = match request.flow_type {
            OAuthFlowType::AuthorizationCode => (
                Some(build_authorization_url(
                    &pending.connection,
                    &state,
                    &code_challenge,
                )?),
                None,
                None,
                None,
                None,
                600,
            ),
            OAuthFlowType::DeviceCode => {
                let device = self.request_device_code(&pending.connection).await?;
                pending.device_code = Some(device.device_code);
                pending.poll_interval_secs = device.interval.unwrap_or(5).max(1);
                pending.expires_at =
                    Utc::now().timestamp() + i64::try_from(device.expires_in).unwrap_or(600);
                pending.next_poll_at = Utc::now().timestamp() + pending.poll_interval_secs as i64;
                (
                    None,
                    Some(device.user_code),
                    device.verification_uri.or(device.verification_url),
                    device.message,
                    Some(pending.poll_interval_secs),
                    device.expires_in,
                )
            }
        };

        if request.flow_type == OAuthFlowType::AuthorizationCode {
            self.oauth_states.insert(state, flow_id.clone());
        }
        self.oauth_flows
            .insert(flow_id.clone(), Arc::new(tokio::sync::Mutex::new(pending)));
        Ok(OAuthFlowStart {
            flow_id,
            authorization_url,
            user_code,
            verification_uri,
            message,
            interval_secs,
            expires_in_secs,
        })
    }

    pub async fn complete_oauth_authorization(
        &self,
        state: &str,
        code: &str,
    ) -> ManageConnectionsResult<OAuthFlowStatus> {
        let (_, flow_id) = self.oauth_states.remove(state).ok_or_else(|| {
            ManageConnectionsError::OAuth("invalid or expired OAuth state".into())
        })?;
        let flow = self
            .oauth_flows
            .get(&flow_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| ManageConnectionsError::OAuth("OAuth flow not found".into()))?;
        let mut flow = flow.lock().await;
        if flow.state != state
            || flow.flow_type != OAuthFlowType::AuthorizationCode
            || flow.status != "pending"
            || flow.expires_at <= Utc::now().timestamp()
        {
            flow.status = "expired".to_string();
            clear_flow_secrets(&mut flow);
            return Err(ManageConnectionsError::OAuth(
                "OAuth flow expired or already used".to_string(),
            ));
        }
        let token = match self.exchange_authorization_code(&flow, code).await {
            Ok(token) => token,
            Err(error) => {
                flow.status = "failed".to_string();
                flow.message = Some("Authorization code exchange failed".to_string());
                clear_flow_secrets(&mut flow);
                return Err(error);
            }
        };
        let connection_id = match self.persist_oauth_connection(&flow, token).await {
            Ok(id) => id,
            Err(error) => {
                flow.status = "failed".to_string();
                flow.message = Some("Provider connection could not be saved".to_string());
                clear_flow_secrets(&mut flow);
                return Err(error);
            }
        };
        flow.status = "connected".to_string();
        flow.message = Some("Provider connected successfully".to_string());
        flow.connection_id = Some(connection_id);
        clear_flow_secrets(&mut flow);
        Ok(flow_status(&flow))
    }

    pub async fn cancel_oauth_authorization(&self, state: &str) {
        let Some((_, flow_id)) = self.oauth_states.remove(state) else {
            return;
        };
        let Some(flow) = self
            .oauth_flows
            .get(&flow_id)
            .map(|entry| entry.value().clone())
        else {
            return;
        };
        let mut flow = flow.lock().await;
        flow.status = "failed".to_string();
        flow.message = Some("Authorization was cancelled".to_string());
        clear_flow_secrets(&mut flow);
    }

    pub async fn oauth_flow_status(
        &self,
        flow_id: &str,
    ) -> ManageConnectionsResult<OAuthFlowStatus> {
        let flow = self
            .oauth_flows
            .get(flow_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| ManageConnectionsError::OAuth("OAuth flow not found".to_string()))?;
        let mut flow = flow.lock().await;
        if flow.expires_at <= Utc::now().timestamp() && flow.status == "pending" {
            flow.status = "expired".to_string();
            flow.message = Some("OAuth authorization expired".to_string());
            self.oauth_states.remove(&flow.state);
            clear_flow_secrets(&mut flow);
        } else if flow.flow_type == OAuthFlowType::DeviceCode
            && flow.status == "pending"
            && Utc::now().timestamp() >= flow.next_poll_at
        {
            self.poll_device_code(&mut flow).await;
        }
        Ok(flow_status(&flow))
    }

    pub async fn refresh_oauth_connection(&self, id: &ConnectionId) -> ManageConnectionsResult<()> {
        self.oauth_token_manager
            .access_token(id, true)
            .await
            .map_err(|error| ManageConnectionsError::OAuth(error.to_string()))?;
        self.refresh_registry().await
    }

    pub async fn disconnect_oauth_connection(
        &self,
        id: &ConnectionId,
    ) -> ManageConnectionsResult<()> {
        let connection = self.get(id).await?.ok_or(RepositoryError::NotFound(*id))?;
        if connection.auth_type != AuthType::OAuth {
            return Err(ManageConnectionsError::OAuth(
                "connection is not OAuth-authenticated".to_string(),
            ));
        }
        if connection.provider_kind == ProviderKind::Gemini {
            if let Credentials::OAuth { refresh_token, .. } = &connection.credentials {
                if let (Some(client), Ok(token)) = (
                    self.oauth_http_client.as_ref(),
                    self.key_manager.decrypt(refresh_token.as_str()),
                ) {
                    let response = client
                        .post("https://oauth2.googleapis.com/revoke")
                        .form(&[("token", token)])
                        .send()
                        .await;
                    if !matches!(response, Ok(response) if response.status().is_success()) {
                        tracing::warn!(
                            connection_id = %id,
                            "Google OAuth revocation failed; disconnecting locally"
                        );
                    }
                }
            }
        }
        // Microsoft Entra does not expose a per-client token revocation
        // endpoint for this delegated flow; deletion removes Rook's local
        // refresh token and unregisters the provider immediately.
        self.delete(id).await
    }

    async fn request_device_code(
        &self,
        connection: &PendingOAuthConnection,
    ) -> ManageConnectionsResult<DeviceCodeResponse> {
        let endpoint =
            device_code_endpoint_with_overrides(connection, &self.oauth_endpoint_overrides)?;
        let Some(client) = self.oauth_http_client.as_ref() else {
            return Err(ManageConnectionsError::OAuth(
                "OAuth HTTP client unavailable".to_string(),
            ));
        };
        let response = client
            .post(endpoint)
            .form(&[
                ("client_id", connection.client_id.as_str()),
                ("scope", connection.scopes.as_str()),
            ])
            .send()
            .await
            .map_err(|_| {
                ManageConnectionsError::OAuth("device authorization request failed".into())
            })?;
        if !response.status().is_success() {
            return Err(ManageConnectionsError::OAuth(
                "device authorization request rejected".to_string(),
            ));
        }
        response.json::<DeviceCodeResponse>().await.map_err(|_| {
            ManageConnectionsError::OAuth("device authorization response invalid".into())
        })
    }

    async fn exchange_authorization_code(
        &self,
        flow: &PendingOAuthFlow,
        code: &str,
    ) -> ManageConnectionsResult<OAuthTokenResponse> {
        if code.is_empty() {
            return Err(ManageConnectionsError::OAuth(
                "authorization code missing".into(),
            ));
        }
        let endpoint =
            token_endpoint_with_overrides(&flow.connection, &self.oauth_endpoint_overrides)?;
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("client_id", flow.connection.client_id.clone()),
            ("code", code.to_string()),
            ("redirect_uri", flow.connection.redirect_uri.clone()),
            ("code_verifier", flow.code_verifier.clone()),
            ("scope", flow.connection.scopes.clone()),
        ];
        if let Some(secret) = &flow.connection.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let Some(client) = self.oauth_http_client.as_ref() else {
            return Err(ManageConnectionsError::OAuth(
                "OAuth HTTP client unavailable".to_string(),
            ));
        };
        let response = client
            .post(endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|_| ManageConnectionsError::OAuth("token exchange failed".into()))?;
        if !response.status().is_success() {
            return Err(ManageConnectionsError::OAuth(
                "authorization code was rejected".to_string(),
            ));
        }
        parse_token_response(response).await
    }

    async fn poll_device_code(&self, flow: &mut PendingOAuthFlow) {
        let Some(device_code) = flow.device_code.clone() else {
            flow.status = "failed".to_string();
            flow.message = Some("Device authorization data is unavailable".to_string());
            clear_flow_secrets(flow);
            return;
        };
        flow.next_poll_at = Utc::now().timestamp() + flow.poll_interval_secs as i64;
        let endpoint =
            match token_endpoint_with_overrides(&flow.connection, &self.oauth_endpoint_overrides) {
                Ok(endpoint) => endpoint,
                Err(_) => {
                    flow.status = "failed".to_string();
                    flow.message =
                        Some("Device authorization configuration is invalid".to_string());
                    clear_flow_secrets(flow);
                    return;
                }
            };
        let form = vec![
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:device_code".to_string(),
            ),
            ("client_id", flow.connection.client_id.clone()),
            ("device_code", device_code),
        ];
        let Some(client) = self.oauth_http_client.as_ref() else {
            flow.status = "failed".to_string();
            flow.message = Some("OAuth HTTP client unavailable".to_string());
            clear_flow_secrets(flow);
            return;
        };
        let response = match client.post(endpoint).form(&form).send().await {
            Ok(response) => response,
            Err(_) => {
                flow.status = "failed".to_string();
                flow.message = Some("Device token polling failed".to_string());
                clear_flow_secrets(flow);
                return;
            }
        };
        if response.status().is_success() {
            match parse_token_response(response).await {
                Ok(token) => match self.persist_oauth_connection(flow, token).await {
                    Ok(id) => {
                        flow.status = "connected".to_string();
                        flow.message = Some("Provider connected successfully".to_string());
                        flow.connection_id = Some(id);
                    }
                    Err(_) => {
                        flow.status = "failed".to_string();
                        flow.message = Some("Provider connection could not be saved".to_string());
                    }
                },
                Err(_) => {
                    flow.status = "failed".to_string();
                    flow.message = Some("Device token response was invalid".to_string());
                }
            }
            clear_flow_secrets(flow);
            return;
        }
        let oauth_error = response
            .json::<OAuthErrorResponse>()
            .await
            .ok()
            .and_then(|error| error.error);
        match oauth_error.as_deref() {
            Some("authorization_pending") => {}
            Some("slow_down") => {
                flow.poll_interval_secs = flow.poll_interval_secs.saturating_add(5);
                flow.next_poll_at = Utc::now().timestamp() + flow.poll_interval_secs as i64;
            }
            Some("authorization_declined") => {
                flow.status = "failed".to_string();
                flow.message = Some("Authorization was declined".to_string());
            }
            Some("expired_token") => {
                flow.status = "expired".to_string();
                flow.message = Some("Device authorization expired".to_string());
            }
            _ => {
                flow.status = "failed".to_string();
                flow.message = Some("Device authorization failed".to_string());
            }
        }
        if flow.status != "pending" {
            clear_flow_secrets(flow);
        }
    }

    async fn persist_oauth_connection(
        &self,
        flow: &PendingOAuthFlow,
        token: OAuthTokenResponse,
    ) -> ManageConnectionsResult<ConnectionId> {
        if token.access_token.is_empty() || token.expires_in == 0 {
            return Err(ManageConnectionsError::OAuth(
                "token response invalid".into(),
            ));
        }
        let refresh_token = token
            .refresh_token
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ManageConnectionsError::OAuth(
                    "provider did not issue a refresh token; reauthorize with offline access"
                        .into(),
                )
            })?;
        let email = self
            .fetch_oauth_email(&flow.connection, &token.access_token)
            .await?;
        let now = Utc::now();
        let credentials = CredentialsInput::OAuth {
            email,
            access_token: token.access_token,
            refresh_token,
            expires_at: now
                .timestamp()
                .saturating_add(i64::try_from(token.expires_in).unwrap_or(i64::MAX)),
            scope: token
                .scope
                .unwrap_or_else(|| flow.connection.scopes.clone()),
            id_token: token.id_token.unwrap_or_default(),
            project_id: flow.connection.project_id.clone(),
            oauth_client_id: flow.connection.client_id.clone(),
            oauth_client_secret: flow.connection.client_secret.clone(),
            oauth_tenant_id: flow.connection.tenant_id.clone(),
        };
        if let Some(id) = flow.connection.existing_connection_id {
            let existing = self.get(&id).await?.ok_or(RepositoryError::NotFound(id))?;
            let updated = self
                .update(
                    &id,
                    UpdateConnectionRequest {
                        expected_updated_at: existing.updated_at,
                        provider_kind: Some(flow.connection.provider_kind),
                        provider_runtime_id: Some(flow.connection.provider_runtime_id.clone()),
                        auth_type: Some(AuthType::OAuth),
                        name: Some(flow.connection.name.clone()),
                        priority: Some(flow.connection.priority),
                        is_active: Some(flow.connection.is_active),
                        credentials: Some(credentials),
                        config: Some(flow.connection.config.clone()),
                    },
                )
                .await?;
            Ok(updated.id)
        } else {
            let connection = self
                .create(CreateConnectionRequest {
                    provider_kind: flow.connection.provider_kind,
                    provider_runtime_id: flow.connection.provider_runtime_id.clone(),
                    auth_type: AuthType::OAuth,
                    name: flow.connection.name.clone(),
                    priority: flow.connection.priority,
                    is_active: flow.connection.is_active,
                    credentials,
                    config: flow.connection.config.clone(),
                })
                .await?;
            Ok(connection.id)
        }
    }

    async fn fetch_oauth_email(
        &self,
        connection: &PendingOAuthConnection,
        access_token: &str,
    ) -> ManageConnectionsResult<String> {
        let endpoint = match connection.provider_kind {
            ProviderKind::AzureOpenAI => "https://graph.microsoft.com/oidc/userinfo",
            ProviderKind::Gemini => "https://openidconnect.googleapis.com/v1/userinfo",
            _ => return Ok(String::new()),
        };
        let Some(client) = self.oauth_http_client.as_ref() else {
            return Ok(String::new());
        };
        let Ok(response) = client.get(endpoint).bearer_auth(access_token).send().await else {
            return Ok(String::new());
        };
        if !response.status().is_success() {
            return Ok(String::new());
        }
        let Ok(info) = response.json::<UserInfoResponse>().await else {
            return Ok(String::new());
        };
        let email = info.email.or(info.preferred_username).unwrap_or_default();
        Ok(if validate_email(&email).is_ok() {
            email
        } else {
            String::new()
        })
    }

    async fn prune_oauth_flows(&self) {
        let now = Utc::now().timestamp();
        let flows: Vec<(String, Arc<tokio::sync::Mutex<PendingOAuthFlow>>)> = self
            .oauth_flows
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        for (id, flow) in flows {
            let expired_or_finished = {
                let flow = flow.lock().await;
                (flow.expires_at <= now || flow.status != "pending").then(|| flow.state.clone())
            };
            if let Some(state) = expired_or_finished {
                self.oauth_flows.remove(&id);
                self.oauth_states.remove(&state);
            }
        }
    }
}
