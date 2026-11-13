use super::*;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
};

struct Fixture {
    root: tempfile::TempDir,
    id: Uuid,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(
                std::env::temp_dir()
                    .canonicalize()
                    .expect("physical parent"),
            )
            .expect("fixture");
        let id = Uuid::nil();
        for name in [".muninn-prepare-once".to_owned(), format!(".lease-{id}")] {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(root.path().join(name))
                .expect("fixture marker");
        }
        let source = root.path().join(format!(".stage-{id}"));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&source)
            .expect("source");
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        for file in manifest.files().iter().take(3) {
            let path = source.join(file.path());
            fs::write(&path, b"abc").expect("prefix bytes");
            fs::set_permissions(path, fs::Permissions::from_mode(0o400)).expect("readonly");
        }
        Self { root, id }
    }

    fn parent(&self) -> ArtifactDir {
        ArtifactDir::open_root(self.root.path()).expect("root")
    }
    fn request(&self) -> RecoveryRequest {
        let metadata = self.root.path().metadata().expect("metadata");
        RecoveryRequest {
            root_device: metadata.dev(),
            root_inode: metadata.ino(),
            source_id: self.id,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fn restore(path: &std::path::Path) {
            if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .expect("restore fixture directories only");
                for entry in fs::read_dir(path).expect("fixture entries") {
                    restore(&entry.expect("entry").path());
                }
            }
        }
        restore(self.root.path());
    }
}

#[test]
fn inspects_only_the_exact_known_prefix_without_mutation() {
    let fixture = Fixture::new();
    let before: std::collections::BTreeSet<_> = fs::read_dir(fixture.root.path())
        .expect("entries")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    let result = RecoverySnapshot::inspect_known_stopped(
        &fixture.parent(),
        PinnedMiniLmManifest::synthetic_for_tests(),
        fixture.request(),
    );
    assert!(result.is_ok());
    let after: std::collections::BTreeSet<_> = fs::read_dir(fixture.root.path())
        .expect("entries")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(before, after);
}

#[test]
fn rejects_unknown_inventory_or_root_identity_without_claiming() {
    for changed in ["identity", "root-entry", "source-entry", "missing-lease"] {
        let fixture = Fixture::new();
        let mut request = fixture.request();
        match changed {
            "identity" => request.root_inode += 1,
            "root-entry" => {
                fs::write(fixture.root.path().join("unknown"), b"keep").expect("fixture")
            }
            "source-entry" => fs::create_dir(
                fixture
                    .root
                    .path()
                    .join(format!(".stage-{}", fixture.id))
                    .join("unknown"),
            )
            .expect("fixture"),
            _ => fs::remove_file(fixture.root.path().join(format!(".lease-{}", fixture.id)))
                .expect("fixture removal"),
        }
        let result = RecoverySnapshot::inspect_known_stopped(
            &fixture.parent(),
            PinnedMiniLmManifest::synthetic_for_tests(),
            request,
        );
        assert!(result.is_err());
        assert!(!fixture.root.path().join(".muninn-recovery-once").exists());
    }
}

#[test]
fn rejects_corrupt_writable_or_linked_prefix_files() {
    for changed in ["corrupt", "writable", "symlink", "hardlink"] {
        let fixture = Fixture::new();
        let outside = tempfile::tempdir().expect("outside fixture");
        let path = fixture
            .root
            .path()
            .join(format!(".stage-{}", fixture.id))
            .join("config.json");
        match changed {
            "corrupt" => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                    .expect("fixture mode");
                fs::write(&path, b"abd").expect("fixture bytes");
                fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).expect("readonly");
            }
            "writable" => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("fixture mode")
            }
            "symlink" => {
                fs::remove_file(&path).expect("fixture removal");
                std::os::unix::fs::symlink(outside.path().join("missing"), &path)
                    .expect("fixture link");
            }
            _ => fs::hard_link(&path, outside.path().join("alias")).expect("fixture hardlink"),
        }
        assert!(RecoverySnapshot::inspect_known_stopped(
            &fixture.parent(),
            PinnedMiniLmManifest::synthetic_for_tests(),
            fixture.request()
        )
        .is_err());
        assert!(!fixture.root.path().join(".muninn-recovery-once").exists());
    }
}

#[test]
fn logical_budget_includes_preserved_files_and_metadata_without_large_allocations() {
    let limit = crate::artifacts::MAX_ARTIFACT_BYTES;
    assert!(snapshot::budget(9, limit - 64 * 1024 - 9).is_ok());
    for (old, new) in [(9, limit - 64 * 1024 - 8), (u64::MAX, 1), (9, 0)] {
        assert_eq!(snapshot::budget(old, new), Err(RecoveryError::Budget));
    }
}

#[test]
fn claims_once_without_changing_the_old_source_or_lease() {
    let fixture = Fixture::new();
    let original = fixture
        .root
        .path()
        .join(format!(".stage-{}", fixture.id))
        .metadata()
        .expect("source identity")
        .ino();
    let claim = RecoveryClaim::claim_known_stopped(
        &fixture.parent(),
        PinnedMiniLmManifest::synthetic_for_tests(),
        fixture.request(),
    );
    assert!(claim.is_ok());
    assert!(fixture.root.path().join(".muninn-recovery-once").is_file());
    assert_eq!(
        fixture
            .root
            .path()
            .join(format!(".stage-{}", fixture.id))
            .metadata()
            .expect("preserved source")
            .ino(),
        original
    );
    assert_eq!(
        fs::read_dir(fixture.root.path()).expect("entries").count(),
        4
    );
    assert!(RecoveryClaim::claim_known_stopped(
        &fixture.parent(),
        PinnedMiniLmManifest::synthetic_for_tests(),
        fixture.request()
    )
    .is_err());
}

#[test]
fn recovery_sync_errors_keep_the_claim_and_old_data_without_a_new_stage() {
    for fail_at in [1, 2] {
        let fixture = Fixture::new();
        let snapshot = RecoverySnapshot::inspect_known_stopped(
            &fixture.parent(),
            PinnedMiniLmManifest::synthetic_for_tests(),
            fixture.request(),
        )
        .expect("snapshot");
        let mut calls = 0;
        let result = snapshot.claim_with_sync(|file| {
            calls += 1;
            if calls == fail_at {
                Err(std::io::Error::other("synthetic recovery sync failure"))
            } else {
                file.sync_all()
            }
        });
        assert!(matches!(
            result,
            Err(RecoveryError::Filesystem(FsError::OutcomeUnknown))
        ));
        assert!(fixture.root.path().join(".muninn-recovery-once").is_file());
        assert_eq!(
            fs::read_dir(fixture.root.path()).expect("entries").count(),
            4
        );
    }
}

#[test]
fn concurrent_recovery_claims_with_independent_snapshots_have_one_winner() {
    let fixture = Fixture::new();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let snapshot = RecoverySnapshot::inspect_known_stopped(
            &fixture.parent(),
            PinnedMiniLmManifest::synthetic_for_tests(),
            fixture.request(),
        )
        .expect("independent snapshot");
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            snapshot.claim()
        }));
    }
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().expect("worker"))
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    assert_eq!(
        fs::read_dir(fixture.root.path()).expect("entries").count(),
        4
    );
}

#[test]
fn changed_inode_or_namespace_is_rejected_before_claiming() {
    for changed in ["namespace", "source", "file", "lease"] {
        let fixture = Fixture::new();
        let outside = tempfile::tempdir().expect("retained old entries");
        let snapshot = RecoverySnapshot::inspect_known_stopped(
            &fixture.parent(),
            PinnedMiniLmManifest::synthetic_for_tests(),
            fixture.request(),
        )
        .expect("snapshot");
        match changed {
            "namespace" => {
                fs::write(fixture.root.path().join("unknown"), b"keep").expect("fixture")
            }
            "source" => {
                let source = fixture.root.path().join(format!(".stage-{}", fixture.id));
                fs::rename(&source, outside.path().join("old-source"))
                    .expect("preserve original inode");
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&source)
                    .expect("replacement");
                for path in [
                    "config.json",
                    "config_sentence_transformers.json",
                    "modules.json",
                ] {
                    fs::write(source.join(path), b"abc").expect("fixture bytes");
                    fs::set_permissions(source.join(path), fs::Permissions::from_mode(0o400))
                        .expect("readonly");
                }
            }
            _ => {
                let path = if changed == "file" {
                    fixture
                        .root
                        .path()
                        .join(format!(".stage-{}", fixture.id))
                        .join("config.json")
                } else {
                    fixture.root.path().join(format!(".lease-{}", fixture.id))
                };
                fs::rename(&path, outside.path().join("old-entry")).expect("preserve old inode");
                fs::write(
                    &path,
                    if changed == "file" {
                        b"abc".as_slice()
                    } else {
                        b""
                    },
                )
                .expect("replacement");
                fs::set_permissions(
                    path,
                    fs::Permissions::from_mode(if changed == "file" { 0o400 } else { 0o600 }),
                )
                .expect("mode");
            }
        }
        assert!(snapshot.claim().is_err());
        assert!(!fixture.root.path().join(".muninn-recovery-once").exists());
    }
}

#[test]
fn post_claim_changes_preserve_the_claim_but_create_no_stage() {
    let fixture = Fixture::new();
    let snapshot = RecoverySnapshot::inspect_known_stopped(
        &fixture.parent(),
        PinnedMiniLmManifest::synthetic_for_tests(),
        fixture.request(),
    )
    .expect("snapshot");
    let mut calls = 0;
    let result = snapshot.claim_with_sync(|file| {
        file.sync_all()?;
        calls += 1;
        if calls == 2 {
            fs::write(fixture.root.path().join("unexpected"), b"keep")?;
        }
        Ok(())
    });
    assert!(result.is_err());
    assert!(fixture.root.path().join(".muninn-recovery-once").is_file());
    assert_eq!(
        fs::read_dir(fixture.root.path()).expect("entries").count(),
        5
    );
}

#[test]
fn consuming_claim_copies_prefix_to_a_distinct_exclusive_stage() {
    let fixture = Fixture::new();
    let old_source = fixture.root.path().join(format!(".stage-{}", fixture.id));
    let original_inode = old_source
        .join("config.json")
        .metadata()
        .expect("old identity")
        .ino();
    let claim = RecoveryClaim::claim_known_stopped(
        &fixture.parent(),
        PinnedMiniLmManifest::synthetic_for_tests(),
        fixture.request(),
    )
    .expect("claim");
    let run = claim.begin();
    assert!(run.is_ok());
    let run = run.expect("copied prefix");
    assert_eq!(run.reused_paths().len(), 3);
    let stages: Vec<_> = fs::read_dir(fixture.root.path())
        .expect("entries")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            path.file_name()
                .expect("name")
                .to_string_lossy()
                .starts_with(".stage-")
        })
        .collect();
    assert_eq!(stages.len(), 2);
    let new_source = stages
        .iter()
        .find(|path| *path != &old_source)
        .expect("new source");
    assert_eq!(
        fs::read(new_source.join("config.json")).expect("copied bytes"),
        b"abc"
    );
    assert_ne!(
        new_source
            .join("config.json")
            .metadata()
            .expect("new inode")
            .ino(),
        original_inode
    );
    assert_eq!(
        old_source
            .join("config.json")
            .metadata()
            .expect("preserved inode")
            .ino(),
        original_inode
    );
    assert!(matches!(
        run.publish(),
        Err(RecoveryError::Cache(CacheError::Incomplete))
    ));
}

#[test]
fn changed_recovery_marker_blocks_copy_without_new_stage_entries() {
    let fixture = Fixture::new();
    let claim = RecoveryClaim::claim_known_stopped(
        &fixture.parent(),
        PinnedMiniLmManifest::synthetic_for_tests(),
        fixture.request(),
    )
    .expect("claim");
    fs::write(
        fixture.root.path().join(".muninn-recovery-once"),
        b"unexpected state",
    )
    .expect("fixture mutation");
    assert!(matches!(claim.begin(), Err(RecoveryError::ChangedState)));
    assert_eq!(
        fs::read_dir(fixture.root.path()).expect("entries").count(),
        4
    );
}

#[test]
fn interrupted_prefix_copy_keeps_claims_old_data_and_partial_new_stage() {
    let fixture = Fixture::new();
    let claim = RecoveryClaim::claim_known_stopped(
        &fixture.parent(),
        PinnedMiniLmManifest::synthetic_for_tests(),
        fixture.request(),
    )
    .expect("claim");
    let mut copied = 0;
    let result = claim.begin_with_copy(|stage, path, file| {
        copied += 1;
        if copied == 2 {
            return Err(RecoveryError::Filesystem(FsError::Io));
        }
        stage
            .stage_artifact(path, file)
            .map_err(RecoveryError::Cache)
    });
    assert!(matches!(
        result,
        Err(RecoveryError::Filesystem(FsError::Io))
    ));
    assert!(fixture.root.path().join(".muninn-recovery-once").is_file());
    assert_eq!(
        fs::read_dir(fixture.root.path())
            .expect("preserved entries")
            .count(),
        6
    );
    let old = fixture.root.path().join(format!(".stage-{}", fixture.id));
    assert_eq!(
        fs::read(old.join("config.json")).expect("old bytes"),
        b"abc"
    );
    assert_eq!(fs::read_dir(old).expect("old source").count(), 3);
}

#[test]
fn recovered_run_publishes_only_after_remaining_artifacts_and_preserves_old_source() {
    let fixture = Fixture::new();
    let manifest = PinnedMiniLmManifest::synthetic_for_tests();
    let parent = fixture.parent();
    let claim = RecoveryClaim::claim_known_stopped(
        &parent,
        PinnedMiniLmManifest::synthetic_for_tests(),
        fixture.request(),
    )
    .expect("claim");
    let mut run = claim.begin().expect("copied prefix");
    assert!(matches!(
        run.stage_artifact("config.json", std::io::Cursor::new(b"abc")),
        Err(RecoveryError::Cache(CacheError::DuplicateArtifact))
    ));
    for file in manifest.files().iter().skip(3) {
        run.stage_artifact(file.path(), std::io::Cursor::new(b"abc"))
            .expect("remaining artifact");
    }
    let cache = run.publish().expect("complete synthetic cache");
    let reopened = VerifiedArtifactCache::open(
        parent
            .open_subdir(cache.bundle_id())
            .expect("published directory"),
        PinnedMiniLmManifest::synthetic_for_tests(),
    )
    .expect("reopen");
    assert_eq!(
        reopened
            .read_artifact("config.json")
            .expect("verified bytes")
            .as_bytes(),
        b"abc"
    );
    let old = fixture.root.path().join(format!(".stage-{}", fixture.id));
    assert_eq!(fs::read_dir(&old).expect("old source intact").count(), 3);
    assert_eq!(
        fs::read(old.join("config.json")).expect("original bytes"),
        b"abc"
    );
    assert_eq!(
        fs::read_dir(fixture.root.path())
            .expect("bounded namespace")
            .count(),
        6
    );
}
