//! Finite operator guard, not general staging admission or crash recovery.

use super::{acl, map_errno, ArtifactDir, FsError};
use rustix::fs::{Dir, Mode, OFlags};
use std::{fs::File, os::unix::fs::MetadataExt};

const MARKER: &str = ".muninn-prepare-once";

impl ArtifactDir {
    /// Permanently consumes an empty root for one cooperating preparation run.
    /// No automatic cleanup or retry is permitted after a failed claim.
    pub fn claim_single_preparation(&self) -> Result<(), FsError> {
        self.claim_with_sync(File::sync_all)
    }

    fn claim_with_sync<S: FnMut(&File) -> std::io::Result<()>>(
        &self,
        mut sync: S,
    ) -> Result<(), FsError> {
        let root = Self::from_directory(self.directory.try_clone().map_err(|_| FsError::Io)?)?;
        if root.directory.metadata().map_err(|_| FsError::Io)?.mode() & 0o7777 != 0o700 {
            return Err(FsError::UnsafeEntry);
        }
        let mut entries = Dir::read_from(&root.directory).map_err(map_errno)?;
        let mut empty = false;
        for _ in 0..3 {
            match entries.read() {
                None => {
                    empty = true;
                    break;
                }
                Some(entry) => {
                    let entry = entry.map_err(map_errno)?;
                    if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
                        return Err(FsError::AlreadyExists);
                    }
                }
            }
        }
        if !empty {
            return Err(FsError::UnsafeEntry);
        }
        let flags = OFlags::RDWR
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK;
        let marker = File::from(
            rustix::fs::openat(&root.directory, MARKER, flags, Mode::RUSR | Mode::WUSR)
                .map_err(map_errno)?,
        );
        let metadata = marker.metadata().map_err(|_| FsError::Io)?;
        if !metadata.is_file()
            || metadata.mode() & 0o7777 != 0o600
            || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(FsError::UnsafeEntry);
        }
        acl::verify(&marker)?;
        sync(&marker).map_err(|_| FsError::OutcomeUnknown)?;
        sync(&root.directory).map_err(|_| FsError::OutcomeUnknown)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture() -> tempfile::TempDir {
        tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(
                std::env::temp_dir()
                    .canonicalize()
                    .expect("physical parent"),
            )
            .expect("fixture")
    }

    #[test]
    fn claims_one_empty_private_root() {
        let root = fixture();
        let directory = ArtifactDir::open_root(root.path()).expect("root");
        assert!(directory.claim_single_preparation().is_ok());
        assert!(root.path().join(MARKER).is_file());
        assert_eq!(
            root.path().join(MARKER).metadata().expect("marker").mode() & 0o777,
            0o600
        );
        assert!(matches!(
            directory.claim_single_preparation(),
            Err(FsError::AlreadyExists)
        ));
    }

    #[test]
    fn preserves_preexisting_entries_and_never_claims_the_root() {
        let root = fixture();
        std::fs::write(root.path().join("keep"), b"unchanged").expect("existing fixture");
        let directory = ArtifactDir::open_root(root.path()).expect("root");
        assert!(matches!(
            directory.claim_single_preparation(),
            Err(FsError::AlreadyExists)
        ));
        assert_eq!(
            std::fs::read(root.path().join("keep")).expect("data"),
            b"unchanged"
        );
        assert!(!root.path().join(MARKER).exists());
    }

    #[test]
    fn concurrent_operators_with_separate_handles_have_one_winner() {
        let root = fixture();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let directory = ArtifactDir::open_root(root.path()).expect("independent handle");
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                directory.claim_single_preparation()
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
    }

    #[test]
    fn claim_remains_on_the_opened_root_after_path_replacement() {
        let outer = fixture();
        let path = outer.path().join("active");
        std::fs::DirBuilder::new().create(&path).expect("fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).expect("mode");
        let directory = ArtifactDir::open_root(&path).expect("original root");
        std::fs::rename(&path, outer.path().join("moved")).expect("fixture rename");
        std::fs::create_dir(&path).expect("replacement");
        directory
            .claim_single_preparation()
            .expect("anchored claim");
        assert!(outer.path().join("moved").join(MARKER).is_file());
        assert!(!path.join(MARKER).exists());
    }

    #[test]
    fn sync_failure_retains_the_visible_claim_and_blocks_retry() {
        for fail_at in [1, 2] {
            let root = fixture();
            let directory = ArtifactDir::open_root(root.path()).expect("root");
            let mut calls = 0;
            let result = directory.claim_with_sync(|file| {
                calls += 1;
                if calls == fail_at {
                    Err(std::io::Error::other("synthetic sync failure"))
                } else {
                    file.sync_all()
                }
            });
            assert!(matches!(result, Err(FsError::OutcomeUnknown)));
            assert!(root.path().join(MARKER).is_file());
            assert!(matches!(
                directory.claim_single_preparation(),
                Err(FsError::AlreadyExists)
            ));
        }
    }
}
