//! Finite operator workflow, not a service or model-readiness claim.

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod supported {
    use muninn_bridge_spike::artifact_fs::recovery::{
        RecoveryClaim, RecoveryError, RecoveryRequest,
    };
    use muninn_bridge_spike::{
        artifact_fs::{
            staging::model::{CacheError, ModelArtifactStage, VerifiedArtifactCache},
            ArtifactDir, FsError,
        },
        artifact_route::http::{ArtifactHttp, TransferError},
        model_manifest::{ManifestError, PinnedMiniLmManifest},
    };
    use sha2::{Digest, Sha256};
    use std::io::{Cursor, Write};
    use std::{ffi::OsString, path::PathBuf};

    #[derive(Debug, PartialEq, Eq)]
    pub enum RunError {
        Usage,
        Filesystem(FsError),
        Manifest(ManifestError),
        Cache(CacheError),
        Http(TransferError),
        Output,
        Recovery(RecoveryError),
    }

    enum Command {
        Prepare(PathBuf),
        Verify(PathBuf, String),
        RecoverKnown(PathBuf, RecoveryRequest),
    }

    fn parse(mut arguments: impl Iterator<Item = OsString>) -> Result<Command, RunError> {
        let mode = arguments.next().ok_or(RunError::Usage)?;
        let root = PathBuf::from(arguments.next().ok_or(RunError::Usage)?);
        if !root.is_absolute() {
            return Err(RunError::Usage);
        }
        match mode.to_str() {
            Some("prepare") if arguments.next().is_none() => Ok(Command::Prepare(root)),
            Some("verify") => {
                let bundle = arguments
                    .next()
                    .ok_or(RunError::Usage)?
                    .into_string()
                    .map_err(|_| RunError::Usage)?;
                if arguments.next().is_some()
                    || bundle.len() != 64
                    || !bundle
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(RunError::Usage);
                }
                Ok(Command::Verify(root, bundle))
            }
            Some("recover-known") => {
                let root_device = parse_number(arguments.next())?;
                let root_inode = parse_number(arguments.next())?;
                let raw = arguments
                    .next()
                    .ok_or(RunError::Usage)?
                    .into_string()
                    .map_err(|_| RunError::Usage)?;
                let source_id = uuid::Uuid::parse_str(&raw).map_err(|_| RunError::Usage)?;
                if source_id.to_string() != raw || arguments.next().is_some() {
                    return Err(RunError::Usage);
                }
                Ok(Command::RecoverKnown(
                    root,
                    RecoveryRequest {
                        root_device,
                        root_inode,
                        source_id,
                    },
                ))
            }
            _ => Err(RunError::Usage),
        }
    }

    fn parse_number(argument: Option<OsString>) -> Result<u64, RunError> {
        let raw = argument
            .ok_or(RunError::Usage)?
            .into_string()
            .map_err(|_| RunError::Usage)?;
        if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(RunError::Usage);
        }
        raw.parse().map_err(|_| RunError::Usage)
    }

    pub async fn run(arguments: impl Iterator<Item = OsString>) -> Result<(), RunError> {
        match parse(arguments)? {
            Command::Prepare(root) => {
                let parent = ArtifactDir::open_root(&root).map_err(RunError::Filesystem)?;
                parent
                    .claim_single_preparation()
                    .map_err(RunError::Filesystem)?;
                let manifest = manifest()?;
                let mut stage =
                    ModelArtifactStage::new(&parent, self::manifest()?).map_err(RunError::Cache)?;
                let http = ArtifactHttp::new().map_err(RunError::Http)?;
                for artifact in manifest.files() {
                    let bytes = http
                        .fetch(&manifest, artifact.path())
                        .await
                        .map_err(RunError::Http)?;
                    let count = bytes.as_bytes().len();
                    let digest = format!("{:x}", Sha256::digest(bytes.as_bytes()));
                    stage
                        .stage_artifact(artifact.path(), Cursor::new(bytes.into_bytes()))
                        .map_err(RunError::Cache)?;
                    emit(
                        serde_json::json!({"event":"artifact_staged", "path":artifact.path(), "bytes":count, "sha256":digest}),
                    )?;
                }
                let published = stage.publish().map_err(RunError::Cache)?;
                let bundle = published.bundle_id().to_owned();
                drop(published);
                // Reopen relative to the original parent descriptor, not ROOT again.
                emit_verified(&open_cache(&parent, &bundle)?, &manifest)
            }
            Command::Verify(root, bundle) => {
                let parent = ArtifactDir::open_root(&root).map_err(RunError::Filesystem)?;
                emit_verified(&open_cache(&parent, &bundle)?, &manifest()?)
            }
            Command::RecoverKnown(root, request) => recover_known(root, request).await,
        }
    }

    async fn recover_known(root: PathBuf, request: RecoveryRequest) -> Result<(), RunError> {
        // The trusted host must already have observed the legacy worker stop.
        // Neither these identity arguments nor a disk marker prove liveness.
        let parent = ArtifactDir::open_root(&root).map_err(RunError::Filesystem)?;
        let manifest = manifest()?;
        let claim = RecoveryClaim::claim_known_stopped(&parent, self::manifest()?, request)
            .map_err(RunError::Recovery)?;
        let mut run = claim.begin().map_err(RunError::Recovery)?;
        let reused: std::collections::BTreeSet<_> = run.reused_paths().iter().cloned().collect();
        let http = ArtifactHttp::new().map_err(RunError::Http)?;
        for artifact in manifest.files() {
            if reused.contains(artifact.path()) {
                emit(
                    serde_json::json!({"event":"artifact_reused_and_staged", "path":artifact.path(),
                    "bytes":artifact.size(), "verified_sha256":artifact.sha256()}),
                )?;
                continue;
            }
            let bytes = http
                .fetch(&manifest, artifact.path())
                .await
                .map_err(RunError::Http)?;
            let count = bytes.as_bytes().len();
            let digest = format!("{:x}", Sha256::digest(bytes.as_bytes()));
            run.stage_artifact(artifact.path(), Cursor::new(bytes.into_bytes()))
                .map_err(RunError::Recovery)?;
            emit(
                serde_json::json!({"event":"artifact_staged", "path":artifact.path(), "bytes":count, "sha256":digest}),
            )?;
        }
        let published = run.publish().map_err(RunError::Recovery)?;
        let bundle = published.bundle_id().to_owned();
        drop(published);
        emit_verified(&open_cache(&parent, &bundle)?, &manifest)
    }

    fn manifest() -> Result<PinnedMiniLmManifest, RunError> {
        PinnedMiniLmManifest::load_pinned(Cursor::new(include_bytes!(
            "../../../../docs/muninn/evidence/artifact-lock.json"
        )))
        .map_err(RunError::Manifest)
    }

    fn open_cache(parent: &ArtifactDir, bundle: &str) -> Result<VerifiedArtifactCache, RunError> {
        let directory = parent.open_subdir(bundle).map_err(RunError::Filesystem)?;
        let cache = VerifiedArtifactCache::open(directory, manifest()?).map_err(RunError::Cache)?;
        if cache.bundle_id() != bundle {
            return Err(RunError::Cache(CacheError::ReceiptMismatch));
        }
        Ok(cache)
    }

    fn emit_verified(
        cache: &VerifiedArtifactCache,
        manifest: &PinnedMiniLmManifest,
    ) -> Result<(), RunError> {
        let mut total = 0_u64;
        for artifact in manifest.files() {
            let bytes = cache
                .read_artifact(artifact.path())
                .map_err(RunError::Cache)?;
            total += u64::try_from(bytes.as_bytes().len()).map_err(|_| RunError::Output)?;
            emit(
                serde_json::json!({"event":"cache_artifact_verified", "path":artifact.path(), "bytes":bytes.as_bytes().len(),
                "sha256":format!("{:x}", Sha256::digest(bytes.as_bytes()))}),
            )?;
        }
        emit(
            serde_json::json!({"event":"bundle_verified", "bundle_id":cache.bundle_id(), "files":manifest.files().len(),
            "artifact_bytes":total, "revision":manifest.revision(), "inference_executed":false}),
        )
    }

    fn emit(value: serde_json::Value) -> Result<(), RunError> {
        let mut output = std::io::stdout().lock();
        serde_json::to_writer(&mut output, &value).map_err(|_| RunError::Output)?;
        writeln!(output).map_err(|_| RunError::Output)?;
        output.flush().map_err(|_| RunError::Output)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn accepts_only_finite_prepare_or_verify_commands() {
            let arguments = ["prepare", "/physical/cache"].map(OsString::from);
            assert!(matches!(
                parse(arguments.into_iter()),
                Ok(Command::Prepare(_))
            ));
            let bundle = "a".repeat(64);
            assert!(matches!(
                parse(
                    ["verify", "/physical/cache", &bundle]
                        .map(OsString::from)
                        .into_iter()
                ),
                Ok(Command::Verify(_, _))
            ));
            for arguments in [
                vec![],
                vec!["prepare"],
                vec!["prepare", "relative"],
                vec!["prepare", "/physical/cache", "extra"],
                vec!["unknown", "/physical/cache"],
                vec!["verify", "/physical/cache", "../escape"],
                vec!["verify", "/physical/cache", "ABC"],
            ] {
                assert!(matches!(
                    parse(arguments.into_iter().map(OsString::from)),
                    Err(RunError::Usage)
                ));
            }
        }

        #[test]
        fn accepts_only_explicit_canonical_known_recovery_identity() {
            let arguments = [
                "recover-known",
                "/physical/cache",
                "123",
                "456",
                "00000000-0000-0000-0000-000000000000",
            ]
            .map(OsString::from);
            let result = parse(arguments.into_iter());
            assert!(matches!(
                result,
                Ok(Command::RecoverKnown(
                    _,
                    RecoveryRequest {
                        root_device: 123,
                        root_inode: 456,
                        ..
                    }
                ))
            ));
            for arguments in [
                vec![
                    "recover-known",
                    "/physical/cache",
                    "-1",
                    "456",
                    "00000000-0000-0000-0000-000000000000",
                ],
                vec![
                    "recover-known",
                    "/physical/cache",
                    "123",
                    "x",
                    "00000000-0000-0000-0000-000000000000",
                ],
                vec![
                    "recover-known",
                    "/physical/cache",
                    "123",
                    "456",
                    "00000000000000000000000000000000",
                ],
                vec![
                    "recover-known",
                    "/physical/cache",
                    "123",
                    "456",
                    "00000000-0000-0000-0000-000000000000",
                    "extra",
                ],
            ] {
                assert!(matches!(
                    parse(arguments.into_iter().map(OsString::from)),
                    Err(RunError::Usage)
                ));
            }
        }

        #[tokio::test]
        async fn recovery_rejects_wrong_root_identity_before_claim_or_network() {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let root = tempfile::Builder::new()
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir_in(
                    std::env::temp_dir()
                        .canonicalize()
                        .expect("physical parent"),
                )
                .expect("fixture");
            let metadata = root.path().metadata().expect("metadata");
            let arguments = [
                OsString::from("recover-known"),
                root.path().as_os_str().to_owned(),
                OsString::from((metadata.dev() + 1).to_string()),
                OsString::from(metadata.ino().to_string()),
                OsString::from(uuid::Uuid::nil().to_string()),
            ];
            assert!(matches!(
                run(arguments.into_iter()).await,
                Err(RunError::Recovery(RecoveryError::ChangedState))
            ));
            assert_eq!(std::fs::read_dir(root.path()).expect("entries").count(), 0);
        }

        #[tokio::test]
        async fn refuses_a_consumed_root_before_any_preparation() {
            use std::os::unix::fs::PermissionsExt;
            let root = tempfile::Builder::new()
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir_in(
                    std::env::temp_dir()
                        .canonicalize()
                        .expect("physical parent"),
                )
                .expect("fixture");
            std::fs::write(root.path().join("keep"), b"unchanged").expect("existing data");
            let result = run([
                OsString::from("prepare"),
                root.path().as_os_str().to_owned(),
            ]
            .into_iter())
            .await;
            assert!(matches!(
                result,
                Err(RunError::Filesystem(
                    muninn_bridge_spike::artifact_fs::FsError::AlreadyExists
                ))
            ));
            assert_eq!(
                std::fs::read(root.path().join("keep")).expect("preserved data"),
                b"unchanged"
            );
            assert_eq!(std::fs::read_dir(root.path()).expect("entries").count(), 1);
        }

        #[tokio::test]
        async fn verify_on_an_empty_root_creates_no_claim_or_staging_entries() {
            use std::os::unix::fs::PermissionsExt;
            let root = tempfile::Builder::new()
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir_in(
                    std::env::temp_dir()
                        .canonicalize()
                        .expect("physical parent"),
                )
                .expect("fixture");
            let result = run([
                OsString::from("verify"),
                root.path().as_os_str().to_owned(),
                OsString::from("a".repeat(64)),
            ]
            .into_iter())
            .await;
            assert!(matches!(
                result,
                Err(RunError::Filesystem(FsError::NotFound))
            ));
            assert_eq!(std::fs::read_dir(root.path()).expect("entries").count(), 0);
            assert_eq!(
                root.path().metadata().expect("mode").permissions().mode() & 0o777,
                0o700
            );
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::main]
async fn main() {
    if let Err(error) = supported::run(std::env::args_os().skip(1)).await {
        eprintln!("prepare_minilm failed: {error:?}");
        std::process::exit(1);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn main() {
    eprintln!("prepare_minilm requires Linux or macOS");
    std::process::exit(2);
}
