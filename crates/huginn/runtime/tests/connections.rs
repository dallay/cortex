use huginn_core::connections::*;
use huginn_runtime::{connection_config::*, connections::*};
use std::collections::BTreeMap;

const SENTINEL: &str = "p02-runtime-sentinel-secret";
async fn secret_contract(store: &dyn SecretStore) {
    let a = SecretReference::new();
    let b = SecretReference::new();
    let secret = Secret::new(SENTINEL.into()).unwrap();
    // Cleanup is attempted even if an assertion or operation fails.
    let result = async {
        store.write(&a, &secret).await?;
        store
            .write(
                &b,
                &Secret::new("isolated-second-test-entry".into()).unwrap(),
            )
            .await?;
        if store.read(&a).await?.expose() != SENTINEL {
            return Err(ConnectionError::SecretStore);
        }
        store.delete(&a).await?;
        if !matches!(store.read(&a).await, Err(ConnectionError::MissingSecret)) {
            return Err(ConnectionError::SecretStore);
        }
        if store.read(&b).await?.expose() != "isolated-second-test-entry" {
            return Err(ConnectionError::SecretStore);
        }
        store.delete(&a).await?;
        Ok::<_, ConnectionError>(())
    }
    .await;
    let a_cleanup = store.delete(&a).await;
    let b_cleanup = store.delete(&b).await;
    result.unwrap();
    a_cleanup.unwrap();
    b_cleanup.unwrap();
}

#[tokio::test]
async fn explicit_ephemeral_contract_and_restart_loss() {
    secret_contract(&EphemeralSecrets::default()).await;
    let reference = SecretReference::new();
    let store = EphemeralSecrets::default();
    store
        .write(&reference, &Secret::new(SENTINEL.into()).unwrap())
        .await
        .unwrap();
    assert_eq!(store.mode(), SecretMode::Ephemeral);
    drop(store);
    assert!(matches!(
        EphemeralSecrets::default().read(&reference).await,
        Err(ConnectionError::MissingSecret)
    ));
}

#[tokio::test]
#[ignore = "requires accessible macOS Keychain or Linux session D-Bus Secret Service; writes isolated test entries"]
async fn os_secret_backend_contract() {
    secret_contract(&OsSecrets).await;
}

#[tokio::test]
async fn metadata_restart_cas_lock_and_sentinel_absence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("providers.db");
    let store = SqliteConnections::open(&path).unwrap();
    assert!(matches!(
        SqliteConnections::open(&path),
        Err(ConnectionError::Conflict)
    ));
    let mut metadata = store.load().await.unwrap();
    let id = uuid::Uuid::new_v4();
    let record = ConnectionRecord {
        id,
        revision: uuid::Uuid::new_v4(),
        settings: ConnectionSettings {
            provider: "secure".into(),
            endpoint: Some("https://example.invalid/v1".into()),
            model: "alpha".into(),
            requires_secret: true,
            allow_loopback_http: false,
        },
        secret: Some(SecretReference::new()),
        mode: SecretMode::Secure,
    };
    metadata.connections.insert(id, record);
    metadata.selected = Some(id);
    metadata.version = 1;
    store.save(0, &metadata).await.unwrap();
    assert_eq!(
        store.save(0, &metadata).await,
        Err(ConnectionError::Conflict)
    );
    drop(store);
    let store = SqliteConnections::open(&path).unwrap();
    assert_eq!(store.load().await.unwrap(), metadata);
    let persisted_bytes = std::fs::read(&path).unwrap();
    assert!(!persisted_bytes
        .windows(SENTINEL.len())
        .any(|window| window == SENTINEL.as_bytes()));
    assert_eq!(
        format!("{:?}", Secret::new(SENTINEL.into()).unwrap()),
        "Secret([REDACTED])"
    );
}

#[cfg(unix)]
#[test]
#[allow(unsafe_code)] // Only async-signal-safe libc calls in the forked child.
fn dropped_metadata_store_releases_lease_while_forked_child_is_alive() {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("providers.db");
    let store = SqliteConnections::open(&path).unwrap();
    let (mut parent, child) = UnixStream::pair().unwrap();
    let child_fd = child.as_raw_fd();
    // Hold the inherited descriptor open to reproduce a concurrent command's
    // fork-to-exec window without relying on subprocess scheduling.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed");
    if pid == 0 {
        let mut release = 0u8;
        unsafe {
            let received = libc::read(child_fd, std::ptr::addr_of_mut!(release).cast(), 1);
            libc::_exit(if received == 1 && release == b'x' {
                0
            } else {
                1
            });
        }
    }
    drop(child);
    let parent_is_exclusive = matches!(
        SqliteConnections::open(&path),
        Err(ConnectionError::Conflict)
    );
    drop(store);
    let reopened = SqliteConnections::open(&path);
    // Reap the child before asserting so a regression cannot orphan it.
    parent.write_all(b"x").unwrap();
    let mut status = 0;
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    assert_eq!(status, 0);
    assert!(parent_is_exclusive, "live store still owns the lease");
    let _store = reopened.expect("drop must release the metadata lease before child exits");
    assert!(matches!(
        SqliteConnections::open(&path),
        Err(ConnectionError::Conflict)
    ));
}

fn explicit(model: &str, env: &str) -> ConfiguredChoice {
    ConfiguredChoice::Explicit(ExplicitConnectionConfig {
        provider: "secure".into(),
        endpoint: Some("https://example.invalid/v1".into()),
        model: model.into(),
        credential_env: Some(env.into()),
        allow_loopback_http: false,
    })
}
#[test]
fn cli_toml_environment_atomic_precedence_matrix() {
    let environment = BTreeMap::from([
        ("CLI_KEY".into(), "cli-key".into()),
        ("TOML_KEY".into(), "toml-key".into()),
        ("ENV_KEY".into(), "environment-key".into()),
        ("AGENT_API_KEY".into(), SENTINEL.into()),
    ]);
    for (cli, toml, env, expected, model, key) in [
        (
            true,
            true,
            true,
            ConnectionProvenance::Cli,
            "cli",
            "cli-key",
        ),
        (
            false,
            true,
            true,
            ConnectionProvenance::Toml,
            "toml",
            "toml-key",
        ),
        (
            false,
            false,
            true,
            ConnectionProvenance::Environment,
            "environment",
            "environment-key",
        ),
    ] {
        let inputs = ConnectionSources {
            cli: cli.then(|| explicit("cli", "CLI_KEY")),
            toml: toml.then(|| explicit("toml", "TOML_KEY")),
            environment: env.then(|| explicit("environment", "ENV_KEY")),
        }
        .resolve(&environment)
        .unwrap();
        let (choice, provenance) = inputs.highest().unwrap();
        assert_eq!(provenance, expected);
        let ConnectionChoice::Explicit { settings, secret } = choice else {
            panic!("expected explicit choice")
        };
        assert_eq!(settings.model, model);
        assert_eq!(secret.unwrap().expose(), key);
    }
    let parsed = ExplicitConnectionConfig::parse_toml("provider='secure'\nmodel='toml'\nendpoint='https://example.invalid/v1'\ncredential_env='TOML_KEY'").unwrap();
    let inputs = ConnectionSources {
        cli: None,
        toml: Some(ConfiguredChoice::Explicit(parsed)),
        environment: Some(explicit("environment", "ENV_KEY")),
    }
    .resolve(&environment)
    .unwrap();
    assert_eq!(inputs.highest().unwrap().1, ConnectionProvenance::Toml);
    // Missing custom credentials cannot inherit either defaults or legacy credentials.
    assert!(matches!(
        ConnectionSources {
            cli: Some(explicit("cli", "UNSET_KEY")),
            toml: Some(explicit("toml", "TOML_KEY")),
            environment: Some(explicit("environment", "ENV_KEY"))
        }
        .resolve(&environment),
        Err(ConnectionError::MissingSecret)
    ));
    let error = ExplicitConnectionConfig::parse_toml(&format!(
        "provider='secure'\nmodel='alpha'\nsecret='{SENTINEL}'"
    ))
    .err()
    .unwrap();
    assert!(!format!("{error:?} {error}").contains(SENTINEL));
    assert!(ConnectionSources {
        cli: None,
        toml: None,
        environment: None
    }
    .resolve(&environment)
    .unwrap()
    .highest()
    .is_none());
    // Credential-bearing URLs cannot be persisted as endpoint metadata.
    for endpoint in [
        format!("https://{SENTINEL}@example.invalid"),
        format!("https://example.invalid?key={SENTINEL}"),
    ] {
        assert!(ExplicitConnectionConfig {
            provider: "secure".into(),
            endpoint: Some(endpoint),
            model: "alpha".into(),
            credential_env: None,
            allow_loopback_http: false,
        }
        .into_choice(&environment)
        .is_err());
    }
}
