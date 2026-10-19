//! Atomic headless CLI/TOML/environment inputs. Lower tiers never fill a choice.
use huginn_core::connections::*;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplicitConnectionConfig {
    pub provider: String,
    pub endpoint: Option<String>,
    pub model: String,
    pub credential_env: Option<String>,
    #[serde(default)]
    pub allow_loopback_http: bool,
}
impl ExplicitConnectionConfig {
    pub fn parse_toml(text: &str) -> ConnectionResult<Self> {
        // Parser source errors include excerpts: never expose them.
        toml::from_str(text).map_err(|_| ConnectionError::InvalidConfiguration)
    }
    pub fn into_choice(
        self,
        environment: &BTreeMap<String, String>,
    ) -> ConnectionResult<ConnectionChoice> {
        let requires_secret = self.credential_env.is_some();
        let secret = self
            .credential_env
            .map(|name| {
                if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return Err(ConnectionError::InvalidConfiguration);
                }
                Secret::new(
                    environment
                        .get(&name)
                        .ok_or(ConnectionError::MissingSecret)?
                        .clone(),
                )
            })
            .transpose()?;
        let settings = ConnectionSettings {
            provider: self.provider,
            endpoint: self.endpoint,
            model: self.model,
            requires_secret,
            allow_loopback_http: self.allow_loopback_http,
        };
        settings.validate()?;
        Ok(ConnectionChoice::Explicit { settings, secret })
    }
}
pub enum ConfiguredChoice {
    Stored(uuid::Uuid),
    Explicit(ExplicitConnectionConfig),
}
impl ConfiguredChoice {
    fn into_choice(
        self,
        environment: &BTreeMap<String, String>,
    ) -> ConnectionResult<ConnectionChoice> {
        match self {
            Self::Stored(id) => Ok(ConnectionChoice::Stored(id)),
            Self::Explicit(config) => config.into_choice(environment),
        }
    }
}
pub struct ConnectionSources {
    pub cli: Option<ConfiguredChoice>,
    pub toml: Option<ConfiguredChoice>,
    /// A chosen environment profile, not field-by-field process defaults.
    pub environment: Option<ConfiguredChoice>,
}
impl ConnectionSources {
    pub fn resolve(self, values: &BTreeMap<String, String>) -> ConnectionResult<ConnectionInputs> {
        // Only inspect the winning tier; malformed lower defaults are irrelevant.
        let (choice, provenance) = if let Some(cli) = self.cli {
            (cli, ConnectionProvenance::Cli)
        } else if let Some(toml) = self.toml {
            (toml, ConnectionProvenance::Toml)
        } else if let Some(env) = self.environment {
            (env, ConnectionProvenance::Environment)
        } else {
            return Ok(ConnectionInputs {
                cli: None,
                toml: None,
                environment: None,
            });
        };
        let choice = choice.into_choice(values)?;
        Ok(match provenance {
            ConnectionProvenance::Cli => ConnectionInputs {
                cli: Some(choice),
                toml: None,
                environment: None,
            },
            ConnectionProvenance::Toml => ConnectionInputs {
                cli: None,
                toml: Some(choice),
                environment: None,
            },
            _ => ConnectionInputs {
                cli: None,
                toml: None,
                environment: Some(choice),
            },
        })
    }
}
