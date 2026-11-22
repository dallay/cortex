//! Run `just muninn-bolt-parity` with a running Docker daemon.
//! Each invocation owns a new container. Alternatively supply MUNINN_FIXTURE_URI,
//! MUNINN_FIXTURE_PASSWORD and MUNINN_FIXTURE_DISPOSABLE=1 for an exclusive fresh
//! database; the caller owns teardown. Never reuse it for a second invocation.
use muninn_neo4j::{Neo4jClient, Neo4jConfig};
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};
use uuid::Uuid;

struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        match Command::new("docker")
            .args(["stop", "-t", "2", &self.0])
            .stdout(Stdio::null())
            .status()
        {
            Ok(status) if status.success() => eprintln!("Container {} stopped", self.0),
            Ok(status) => eprintln!("docker stop {} returned {status}", self.0),
            Err(error) => eprintln!("docker stop {} failed: {error}", self.0),
        }
    }
}

pub struct DockerFixture {
    _container: Option<Container>,
    pub config: Neo4jConfig,
}

impl DockerFixture {
    pub fn start(uri_variable: &str) -> Option<Self> {
        let (uri, password, container) = if let Ok(uri) =
            std::env::var(uri_variable).or_else(|_| std::env::var("MUNINN_FIXTURE_URI"))
        {
            assert_eq!(
                std::env::var("MUNINN_FIXTURE_DISPOSABLE").as_deref(),
                Ok("1"),
                "external fixture must be fresh, disposable, and exclusive to this test"
            );
            (
                uri,
                std::env::var("MUNINN_FIXTURE_PASSWORD").expect("fixture password required"),
                None,
            )
        } else {
            if !Command::new("docker")
                .arg("info")
                .output()
                .is_ok_and(|out| out.status.success())
            {
                eprintln!("Skipping live test: start Docker or supply a disposable Neo4j fixture");
                return None;
            }
            let password = format!("p_{}", Uuid::new_v4());
            let mut auth = tempfile::NamedTempFile::new().expect("create private Docker env file");
            let auth_variable = ["NEO4J_", "AUTH"].concat();
            writeln!(auth, "{auth_variable}=neo4j/{password}\nNEO4J_dbms_security_auth__minimum__password__length=4\nNEO4J_db_transaction_timeout=5s\nNEO4J_db_lock_acquisition_timeout=5s\nNEO4J_db_transaction_monitor_check_interval=500ms").unwrap();
            let output = Command::new("docker")
                .args([
                    "run",
                    "-d",
                    "--rm",
                    "--publish",
                    "127.0.0.1::7687",
                    "--env-file",
                ])
                .arg(auth.path())
                .arg("neo4j:5.26-community")
                .output()
                .expect("launch docker");
            assert!(output.status.success(), "docker run failed");
            let id = String::from_utf8(output.stdout).unwrap().trim().to_owned();
            assert!((12..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit()));
            let container = Container(id);
            let port = Command::new("docker")
                .args(["port", &container.0, "7687/tcp"])
                .output()
                .unwrap();
            assert!(port.status.success(), "docker port failed");
            let port = String::from_utf8(port.stdout)
                .unwrap()
                .trim()
                .rsplit_once(':')
                .and_then(|(_, port)| port.parse::<std::num::NonZeroU16>().ok())
                .expect("valid published port");
            (
                format!("bolt://127.0.0.1:{port}"),
                password,
                Some(container),
            )
        };
        Some(Self {
            _container: container,
            config: Neo4jConfig {
                uri,
                password,
                username: "neo4j".into(),
                ..Default::default()
            },
        })
    }

    pub async fn connect_fresh(&self) -> Neo4jClient {
        for _ in 0..60 {
            if let Ok(client) = Neo4jClient::connect(&self.config).await {
                if let Ok(rows) = client
                    .execute("MATCH (n) RETURN count(n) AS nodes", vec![])
                    .await
                {
                    assert_eq!(
                        rows[0].get::<i64>("nodes").unwrap(),
                        0,
                        "fixture must start empty"
                    );
                    let schema = client
                        .execute(
                            "SHOW INDEXES YIELD type WHERE type <> 'LOOKUP' RETURN type",
                            vec![],
                        )
                        .await
                        .unwrap();
                    assert!(
                        schema.is_empty(),
                        "fixture must start without application indexes"
                    );
                    return client;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        panic!("Neo4j did not become ready")
    }
}
