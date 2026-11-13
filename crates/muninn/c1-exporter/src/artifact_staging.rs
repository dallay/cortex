//! Exclusive source ownership and directory visibility, not verified model readiness.

use super::{map_errno, ArtifactDir, FsError};
use rustix::fs::{Mode, OFlags};
use std::{fs::File, os::unix::fs::MetadataExt};
use uuid::Uuid;
#[path = "artifact_model_stage.rs"]
pub mod model;

/// Lease markers intentionally persist; this primitive performs no automatic cleanup.
pub struct ExclusiveStage {
    parent: File,
    directory: ArtifactDir,
    source_name: String,
    _reservation: File,
}

impl ExclusiveStage {
    /// Creates a fresh private source and permanent exclusive name reservation.
    /// Dropping the owner intentionally leaves both entries for explicit maintenance.
    pub fn begin(parent: &ArtifactDir) -> Result<Self, FsError> {
        Self::begin_with_id(parent, Uuid::new_v4())
    }

    fn begin_with_id(parent: &ArtifactDir, id: Uuid) -> Result<Self, FsError> {
        let checked =
            ArtifactDir::from_directory(parent.directory.try_clone().map_err(|_| FsError::Io)?)?;
        let source_name = format!(".stage-{id}");
        let lease_name = format!(".lease-{id}");
        let flags = OFlags::RDWR
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::CLOEXEC
            | OFlags::NOFOLLOW
            | OFlags::NONBLOCK;
        let reservation = File::from(
            rustix::fs::openat(
                &checked.directory,
                &lease_name,
                flags,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(map_errno)?,
        );
        let metadata = reservation.metadata().map_err(|_| FsError::Io)?;
        if !metadata.is_file()
            || metadata.mode() & 0o7777 != 0o600
            || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(FsError::UnsafeEntry);
        }
        super::acl::verify(&reservation)?;
        rustix::fs::mkdirat(&checked.directory, &source_name, Mode::RWXU).map_err(map_errno)?;
        let directory = checked.open_subdir(&source_name)?;
        Ok(Self {
            parent: checked.directory,
            directory,
            source_name,
            _reservation: reservation,
        })
    }

    pub const fn directory(&self) -> &ArtifactDir {
        &self.directory
    }

    /// Publishes directory visibility only, never a complete or durable model.
    /// OutcomeUnknown means rename succeeded but the subsequent parent sync failed.
    pub fn publish_directory(self, target: &str) -> Result<ArtifactDir, FsError> {
        self.publish_with_sync(target, File::sync_all, File::sync_all)
    }

    fn publish_with_sync<B, A>(
        self,
        target: &str,
        before: B,
        after: A,
    ) -> Result<ArtifactDir, FsError>
    where
        B: FnOnce(&File) -> std::io::Result<()>,
        A: FnOnce(&File) -> std::io::Result<()>,
    {
        if !crate::artifacts::is_lower_hex(target, 64) {
            return Err(FsError::InvalidPath);
        }
        let parent =
            ArtifactDir::from_directory(self.parent.try_clone().map_err(|_| FsError::Io)?)?;
        let named = parent.open_subdir(&self.source_name)?;
        let owned = self
            .directory
            .directory
            .metadata()
            .map_err(|_| FsError::Io)?;
        let current = named.directory.metadata().map_err(|_| FsError::Io)?;
        if owned.dev() != current.dev() || owned.ino() != current.ino() {
            return Err(FsError::UnsafeEntry);
        }
        before(&self.directory.directory).map_err(|_| FsError::Io)?;
        rustix::fs::renameat_with(
            &self.parent,
            &self.source_name,
            &self.parent,
            target,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(map_errno)?;
        after(&self.parent).map_err(|_| FsError::OutcomeUnknown)?;
        Ok(self.directory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn fixture() -> tempfile::TempDir {
        tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(
                std::env::temp_dir()
                    .canonicalize()
                    .expect("physical temp parent"),
            )
            .expect("private fixture")
    }

    fn write_marker(stage: &ExclusiveStage, data: &[u8]) {
        use std::io::Write;
        let descriptor = rustix::fs::openat(
            &stage.directory.directory,
            "marker",
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::RUSR | Mode::WUSR,
        )
        .expect("fixture marker");
        let mut file = File::from(descriptor);
        file.write_all(data).expect("marker data");
        file.sync_all()
            .expect("fixture data sync, separate from directory sync");
    }

    #[test]
    fn creates_a_private_descriptor_bound_stage() {
        let root = fixture();
        let parent = ArtifactDir::open_root(root.path()).expect("root");
        let stage = ExclusiveStage::begin(&parent);
        assert!(stage.is_ok());
        let stage = stage.expect("stage");
        assert_eq!(
            stage
                .directory
                .directory
                .metadata()
                .expect("metadata")
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            stage
                ._reservation
                .metadata()
                .expect("lease metadata")
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn rejects_a_second_owner_for_the_same_source_name() {
        let root = fixture();
        let parent = ArtifactDir::open_root(root.path()).expect("root");
        let _owner = ExclusiveStage::begin_with_id(&parent, Uuid::nil()).expect("first owner");
        assert!(matches!(
            ExclusiveStage::begin_with_id(&parent, Uuid::nil()),
            Err(FsError::AlreadyExists)
        ));
    }

    #[test]
    fn never_replaces_an_existing_empty_destination() {
        let root = fixture();
        let parent = ArtifactDir::open_root(root.path()).expect("root");
        let target = "a".repeat(64);
        std::fs::create_dir(root.path().join(&target)).expect("existing target");
        let before = root
            .path()
            .join(&target)
            .metadata()
            .expect("metadata")
            .ino();
        let stage = ExclusiveStage::begin(&parent).expect("stage");
        let source = stage.source_name.clone();
        assert!(matches!(
            stage.publish_directory(&target),
            Err(FsError::AlreadyExists)
        ));
        assert_eq!(
            root.path()
                .join(&target)
                .metadata()
                .expect("metadata")
                .ino(),
            before
        );
        assert!(root.path().join(source).is_dir());
    }

    #[test]
    fn pre_publication_sync_failure_leaves_source_and_target_unchanged() {
        let root = fixture();
        let parent = ArtifactDir::open_root(root.path()).expect("root");
        let stage = ExclusiveStage::begin(&parent).expect("stage");
        let source = stage.source_name.clone();
        let target = "b".repeat(64);
        let result = stage.publish_with_sync(
            &target,
            |_| Err(std::io::Error::other("synthetic pre-sync failure")),
            File::sync_all,
        );
        assert!(matches!(result, Err(FsError::Io)));
        assert!(root.path().join(source).is_dir());
        assert!(!root.path().join(target).exists());
    }

    #[test]
    fn post_publication_sync_failure_reports_uncertain_visible_outcome() {
        let root = fixture();
        let parent = ArtifactDir::open_root(root.path()).expect("root");
        let stage = ExclusiveStage::begin(&parent).expect("stage");
        let source = stage.source_name.clone();
        let target = "c".repeat(64);
        let result = stage.publish_with_sync(&target, File::sync_all, |_| {
            Err(std::io::Error::other("synthetic post-sync failure"))
        });
        assert!(matches!(result, Err(FsError::OutcomeUnknown)));
        assert!(root.path().join(&target).is_dir());
        assert!(!root.path().join(source).exists());
    }

    #[test]
    fn rejects_a_source_entry_that_no_longer_names_the_owned_inode() {
        let root = fixture();
        let parent = ArtifactDir::open_root(root.path()).expect("root");
        let stage = ExclusiveStage::begin(&parent).expect("stage");
        let source = root.path().join(&stage.source_name);
        std::fs::rename(&source, root.path().join("moved-owned-source")).expect("fixture rename");
        std::fs::create_dir(&source).expect("replacement");
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o700)).expect("mode");
        let target = "d".repeat(64);
        assert!(matches!(
            stage.publish_directory(&target),
            Err(FsError::UnsafeEntry)
        ));
        assert!(!root.path().join(target).exists());
    }

    #[test]
    fn publishes_the_owned_inode_and_retains_its_reservation() {
        let root = fixture();
        let parent = ArtifactDir::open_root(root.path()).expect("root");
        let id = Uuid::nil();
        let stage = ExclusiveStage::begin_with_id(&parent, id).expect("stage");
        write_marker(&stage, b"source");
        let inode = stage
            .directory
            .directory
            .metadata()
            .expect("metadata")
            .ino();
        let source = stage.source_name.clone();
        let target = "e".repeat(64);
        let published = stage
            .publish_directory(&target)
            .expect("visible publication");
        assert_eq!(
            published.directory.metadata().expect("metadata").ino(),
            inode
        );
        assert_eq!(
            std::fs::read(root.path().join(target).join("marker")).expect("marker"),
            b"source"
        );
        assert!(!root.path().join(source).exists());
        assert!(matches!(
            ExclusiveStage::begin_with_id(&parent, id),
            Err(FsError::AlreadyExists)
        ));
    }

    #[test]
    fn source_name_stays_reserved_when_its_directory_is_absent_and_owner_drops() {
        let root = fixture();
        let parent = ArtifactDir::open_root(root.path()).expect("root");
        let id = Uuid::nil();
        let stage = ExclusiveStage::begin_with_id(&parent, id).expect("stage");
        std::fs::rename(
            root.path().join(&stage.source_name),
            root.path().join("moved-source"),
        )
        .expect("fixture rename simulates the post-rename window");
        assert!(matches!(
            ExclusiveStage::begin_with_id(&parent, id),
            Err(FsError::AlreadyExists)
        ));
        drop(stage);
        assert!(matches!(
            ExclusiveStage::begin_with_id(&parent, id),
            Err(FsError::AlreadyExists)
        ));
    }

    #[test]
    fn preserves_existing_files_populated_directories_and_dangling_links() {
        use std::os::unix::fs::symlink;
        for kind in ["file", "populated", "link"] {
            let root = fixture();
            let parent = ArtifactDir::open_root(root.path()).expect("root");
            let target = "f".repeat(64);
            let destination = root.path().join(&target);
            match kind {
                "file" => std::fs::write(&destination, b"keep").expect("file"),
                "populated" => {
                    std::fs::create_dir(&destination).expect("directory");
                    std::fs::write(destination.join("keep"), b"keep").expect("data");
                }
                _ => symlink("missing", &destination).expect("dangling link"),
            }
            let inode = std::fs::symlink_metadata(&destination)
                .expect("metadata")
                .ino();
            let stage = ExclusiveStage::begin(&parent).expect("stage");
            write_marker(&stage, b"source");
            let source = stage.source_name.clone();
            assert!(stage.publish_directory(&target).is_err());
            assert_eq!(
                std::fs::symlink_metadata(&destination)
                    .expect("metadata")
                    .ino(),
                inode
            );
            assert_eq!(
                std::fs::read(root.path().join(source).join("marker")).expect("source preserved"),
                b"source"
            );
            match kind {
                "file" => assert_eq!(std::fs::read(&destination).expect("data"), b"keep"),
                "populated" => assert_eq!(
                    std::fs::read(destination.join("keep")).expect("data"),
                    b"keep"
                ),
                _ => assert_eq!(
                    std::fs::read_link(&destination).expect("link"),
                    std::path::PathBuf::from("missing")
                ),
            }
        }
    }

    #[test]
    fn concurrent_publishers_have_exactly_one_complete_winner() {
        let root = fixture();
        let parent = ArtifactDir::open_root(root.path()).expect("root");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut workers = Vec::new();
        for label in [b"alpha".as_slice(), b"beta".as_slice()] {
            let stage = ExclusiveStage::begin(&parent).expect("stage");
            write_marker(&stage, label);
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                stage.publish_directory(&"a".repeat(64)).map(|_| label)
            }));
        }
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker"))
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(FsError::AlreadyExists)))
                .count(),
            1
        );
        let winner = results.into_iter().find_map(Result::ok).expect("winner");
        assert_eq!(
            std::fs::read(root.path().join("a".repeat(64)).join("marker")).expect("winner data"),
            winner
        );
    }

    #[test]
    fn invalid_target_is_rejected_before_sync_or_publication() {
        let root = fixture();
        let parent = ArtifactDir::open_root(root.path()).expect("root");
        for target in ["", "../escape", &"a".repeat(63), &"A".repeat(64)] {
            let stage = ExclusiveStage::begin(&parent).expect("stage");
            let source = stage.source_name.clone();
            let result = stage.publish_with_sync(
                target,
                |_| panic!("invalid target reached sync"),
                File::sync_all,
            );
            assert!(matches!(result, Err(FsError::InvalidPath)));
            assert!(root.path().join(source).is_dir());
        }
    }

    #[test]
    fn rechecks_parent_and_source_policy_before_publication() {
        for changed in ["parent", "source"] {
            let root = fixture();
            let parent = ArtifactDir::open_root(root.path()).expect("root");
            let stage = ExclusiveStage::begin(&parent).expect("stage");
            let path = if changed == "parent" {
                root.path().to_path_buf()
            } else {
                root.path().join(&stage.source_name)
            };
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("fixture change");
            let result = stage.publish_directory(&"b".repeat(64));
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .expect("restore fixture");
            assert!(matches!(result, Err(FsError::UnsafeEntry)));
            assert!(!root.path().join("b".repeat(64)).exists());
        }
    }

    #[test]
    fn publication_remains_anchored_after_parent_path_replacement() {
        let root = fixture();
        let path = root.path().join("active");
        std::fs::create_dir(&path).expect("fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).expect("mode");
        let parent = ArtifactDir::open_root(&path).expect("root");
        let stage = ExclusiveStage::begin(&parent).expect("stage");
        write_marker(&stage, b"anchored");
        std::fs::rename(&path, root.path().join("moved")).expect("fixture rename");
        std::fs::create_dir(&path).expect("replacement");
        let target = "c".repeat(64);
        assert!(stage.publish_directory(&target).is_ok());
        assert_eq!(
            std::fs::read(root.path().join("moved").join(&target).join("marker"))
                .expect("anchored data"),
            b"anchored"
        );
        assert!(!path.join(target).exists());
    }
}
