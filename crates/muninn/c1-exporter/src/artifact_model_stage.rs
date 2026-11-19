//! Pinned artifact integrity, not model inference or a permanent immutability lease.

use super::super::{acl, map_errno, validate_name};
use super::{ArtifactDir, ExclusiveStage, FsError};
use crate::model_manifest::{ManifestError, PinnedMiniLmManifest};
use rustix::fs::{Mode, OFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
};

const RECEIPT_NAME: &str = "_muninn_artifacts.json";

#[derive(Debug, PartialEq, Eq)]
pub enum CacheError {
    Manifest(ManifestError),
    Filesystem(FsError),
    DuplicateArtifact,
    Incomplete,
    Poisoned,
    ReceiptMismatch,
    TooLarge,
}

pub struct ModelArtifactStage {
    manifest: PinnedMiniLmManifest,
    stage: ExclusiveStage,
    completed: BTreeSet<String>,
    failed: bool,
}

pub struct VerifiedArtifactCache {
    directory: ArtifactDir,
    manifest: PinnedMiniLmManifest,
    bundle_id: String,
}

impl VerifiedArtifactCache {
    pub fn open(
        directory: ArtifactDir,
        manifest: PinnedMiniLmManifest,
    ) -> Result<Self, CacheError> {
        check_readonly(&directory.directory, 0o500)?;
        let expected = receipt_bytes(&manifest)?;
        let bundle_id = format!("{:x}", Sha256::digest(&expected));
        let file = directory
            .open_file(RECEIPT_NAME)
            .map_err(CacheError::Filesystem)?;
        check_readonly(&file, 0o400)?;
        let checked = crate::artifacts::load_verified(
            file,
            u64::try_from(expected.len()).map_err(|_| CacheError::TooLarge)?,
            &bundle_id,
        )
        .map_err(|_| CacheError::ReceiptMismatch)?;
        if checked.as_bytes() != expected {
            return Err(CacheError::ReceiptMismatch);
        }
        for artifact in manifest.files() {
            manifest
                .verify_artifact(
                    artifact.path(),
                    open_readonly_artifact(&directory, artifact.path())?,
                )
                .map_err(CacheError::Manifest)?;
        }
        Ok(Self {
            directory,
            manifest,
            bundle_id,
        })
    }

    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    pub fn read_artifact(
        &self,
        path: &str,
    ) -> Result<crate::artifacts::VerifiedArtifact, CacheError> {
        if !self.manifest.files().iter().any(|file| file.path() == path) {
            return Err(CacheError::Manifest(ManifestError::UnknownArtifact));
        }
        self.manifest
            .verify_artifact(path, open_readonly_artifact(&self.directory, path)?)
            .map_err(CacheError::Manifest)
    }
}

impl ModelArtifactStage {
    pub fn new(parent: &ArtifactDir, manifest: PinnedMiniLmManifest) -> Result<Self, CacheError> {
        let total = manifest
            .files()
            .iter()
            .try_fold(0_u64, |sum, file| sum.checked_add(file.size()))
            .ok_or(CacheError::TooLarge)?;
        if total == 0 || total > crate::artifacts::MAX_ARTIFACT_BYTES {
            return Err(CacheError::TooLarge);
        }
        Ok(Self {
            manifest,
            stage: ExclusiveStage::begin(parent).map_err(CacheError::Filesystem)?,
            completed: BTreeSet::new(),
            failed: false,
        })
    }

    pub fn stage_artifact<R: Read>(&mut self, path: &str, reader: R) -> Result<(), CacheError> {
        self.stage_with_sync(path, reader, sync_file)
    }

    fn stage_with_sync<R: Read, S: FnOnce(&File) -> Result<(), FsError>>(
        &mut self,
        path: &str,
        reader: R,
        sync: S,
    ) -> Result<(), CacheError> {
        self.stage_with_io(path, reader, |file, bytes| file.write_all(bytes), sync)
    }

    fn stage_with_io<R, W, S>(
        &mut self,
        path: &str,
        reader: R,
        write: W,
        sync: S,
    ) -> Result<(), CacheError>
    where
        R: Read,
        W: FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
        S: FnOnce(&File) -> Result<(), FsError>,
    {
        if self.failed {
            return Err(CacheError::Poisoned);
        }
        if self.completed.contains(path) {
            return Err(CacheError::DuplicateArtifact);
        }
        let verified = self
            .manifest
            .verify_artifact(path, reader)
            .map_err(CacheError::Manifest)?;
        self.failed = true;
        self.write_artifact(path, verified.as_bytes(), write, sync)?;
        self.completed.insert(path.to_owned());
        self.failed = false;
        Ok(())
    }

    fn write_artifact<W, S>(
        &self,
        path: &str,
        bytes: &[u8],
        write: W,
        sync: S,
    ) -> Result<(), CacheError>
    where
        W: FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
        S: FnOnce(&File) -> Result<(), FsError>,
    {
        let root =
            ArtifactDir::from_directory(self.stage.directory.directory.try_clone().map_err(
                |error| {
                    #[cfg(test)]
                    super::super::diagnostics::report("stage.clone_directory", &error);
                    #[cfg(not(test))]
                    let _ = error;
                    CacheError::Filesystem(FsError::Io)
                },
            )?)
            .map_err(CacheError::Filesystem)?;
        let (directory, name) = match path.split_once('/') {
            Some((folder, name)) => {
                validate_name(folder).map_err(CacheError::Filesystem)?;
                rustix::fs::mkdirat(&root.directory, folder, Mode::RWXU)
                    .map_err(|error| CacheError::Filesystem(map_errno(error)))?;
                (
                    root.open_subdir(folder).map_err(CacheError::Filesystem)?,
                    name,
                )
            }
            None => (root, path),
        };
        let mut file = create_new(&directory, name).map_err(CacheError::Filesystem)?;
        write(&mut file, bytes).map_err(|error| {
            #[cfg(test)]
            super::super::diagnostics::report("stage.write", &error);
            #[cfg(not(test))]
            let _ = error;
            CacheError::Filesystem(FsError::Io)
        })?;
        file.rewind().map_err(|error| {
            #[cfg(test)]
            super::super::diagnostics::report("stage.rewind", &error);
            #[cfg(not(test))]
            let _ = error;
            CacheError::Filesystem(FsError::Io)
        })?;
        self.manifest
            .verify_artifact(path, &mut file)
            .map_err(CacheError::Manifest)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o400))
            .map_err(|error| {
                #[cfg(test)]
                super::super::diagnostics::report("stage.chmod_file", &error);
                #[cfg(not(test))]
                let _ = error;
                CacheError::Filesystem(FsError::Io)
            })?;
        sync(&file).map_err(CacheError::Filesystem)?;
        Ok(())
    }

    pub fn publish(self) -> Result<VerifiedArtifactCache, CacheError> {
        self.publish_with_freeze(super::super::freeze_directory)
    }

    /// Publishes the staged bundle and freezes the published entry.
    ///
    /// The freeze callback is a seam: a failure after the rename leaves the
    /// final-name entry writable, and the next publication of the same manifest
    /// must recover it.
    fn publish_with_freeze<F>(self, freeze: F) -> Result<VerifiedArtifactCache, CacheError>
    where
        F: FnOnce(&ArtifactDir) -> Result<(), FsError>,
    {
        self.require_complete()?;
        verify_files(&self.stage.directory, &self.manifest)?;
        let receipt = receipt_bytes(&self.manifest)?;
        let bundle_id = format!("{:x}", Sha256::digest(&receipt));
        {
            let mut file =
                create_new(&self.stage.directory, RECEIPT_NAME).map_err(CacheError::Filesystem)?;
            file.write_all(&receipt).map_err(|error| {
                #[cfg(test)]
                super::super::diagnostics::report("publish.write_receipt", &error);
                #[cfg(not(test))]
                let _ = error;
                CacheError::Filesystem(FsError::Io)
            })?;
            file.rewind().map_err(|error| {
                #[cfg(test)]
                super::super::diagnostics::report("publish.rewind_receipt", &error);
                #[cfg(not(test))]
                let _ = error;
                CacheError::Filesystem(FsError::Io)
            })?;
            crate::artifacts::load_verified(
                &mut file,
                u64::try_from(receipt.len()).map_err(|_| CacheError::TooLarge)?,
                &format!("{:x}", Sha256::digest(&receipt)),
            )
            .map_err(|_| CacheError::ReceiptMismatch)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o400))
                .map_err(|error| {
                    #[cfg(test)]
                    super::super::diagnostics::report("publish.chmod_receipt", &error);
                    #[cfg(not(test))]
                    let _ = error;
                    CacheError::Filesystem(FsError::Io)
                })?;
            sync_file(&file).map_err(CacheError::Filesystem)?;
        }
        {
            // The child directory is never renamed, so freezing it before
            // publication keeps its contents immutable from the moment the
            // bundle becomes visible.
            let child = self
                .stage
                .directory
                .open_subdir("1_Pooling")
                .map_err(CacheError::Filesystem)?;
            super::super::freeze_directory(&child).map_err(CacheError::Filesystem)?;
        }
        // The staged directory cannot be frozen before the rename: macOS 15
        // denies renaming a directory whose mode lacks write permission. The
        // published entry is frozen right after the rename instead.
        let manifest = self.manifest;
        let expected = receipt;
        let digest = bundle_id.clone();
        let directory = self
            .stage
            .publish_frozen_directory_with(&bundle_id, freeze, |existing| {
                matches_manifest(existing, &manifest, &expected, &digest)
            })
            .map_err(CacheError::Filesystem)?;
        Ok(VerifiedArtifactCache {
            directory,
            manifest,
            bundle_id,
        })
    }

    fn require_complete(&self) -> Result<(), CacheError> {
        if self.failed {
            return Err(CacheError::Poisoned);
        }
        let expected: BTreeSet<_> = self
            .manifest
            .files()
            .iter()
            .map(|file| file.path())
            .collect();
        if self
            .completed
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != expected
        {
            return Err(CacheError::Incomplete);
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct ArtifactReceipt<'a> {
    schema_version: u32,
    model_id: &'a str,
    revision: &'a str,
    files: Vec<ReceiptFile<'a>>,
}

#[derive(Serialize)]
struct ReceiptFile<'a> {
    path: &'a str,
    size: u64,
    sha256: &'a str,
}

fn receipt_bytes(manifest: &PinnedMiniLmManifest) -> Result<Vec<u8>, CacheError> {
    let mut files: Vec<_> = manifest
        .files()
        .iter()
        .map(|file| ReceiptFile {
            path: file.path(),
            size: file.size(),
            sha256: file.sha256(),
        })
        .collect();
    files.sort_unstable_by_key(|file| file.path);
    serde_json::to_vec(&ArtifactReceipt {
        schema_version: 1,
        model_id: crate::model_manifest::MODEL_ID,
        revision: manifest.revision(),
        files,
    })
    .map_err(|_| CacheError::ReceiptMismatch)
}

/// Reports whether an existing final-name entry is exactly the requested
/// manifest: its receipt bytes must match the expected receipt and every
/// artifact must still match the pinned manifest.
///
/// A failure to verify is never an adoption: the entry is left untouched.
fn matches_manifest(
    existing: &ArtifactDir,
    manifest: &PinnedMiniLmManifest,
    expected: &[u8],
    digest: &str,
) -> bool {
    let Ok(file) = existing.open_file(RECEIPT_NAME) else {
        return false;
    };
    let Ok(size) = u64::try_from(expected.len()) else {
        return false;
    };
    let Ok(checked) = crate::artifacts::load_verified(&file, size, digest) else {
        return false;
    };
    if checked.as_bytes() != expected {
        return false;
    }
    verify_files(existing, manifest).is_ok()
}

fn open_artifact(directory: &ArtifactDir, path: &str) -> Result<File, CacheError> {
    match path.split_once('/') {
        Some((folder, name)) => directory
            .open_subdir(folder)
            .and_then(|child| child.open_file(name)),
        None => directory.open_file(path),
    }
    .map_err(CacheError::Filesystem)
}

fn verify_files(
    directory: &ArtifactDir,
    manifest: &PinnedMiniLmManifest,
) -> Result<(), CacheError> {
    for artifact in manifest.files() {
        let file = open_artifact(directory, artifact.path())?;
        check_readonly(&file, 0o400)?;
        manifest
            .verify_artifact(artifact.path(), file)
            .map_err(CacheError::Manifest)?;
    }
    Ok(())
}

fn check_readonly(file: &File, mode: u32) -> Result<(), CacheError> {
    let metadata = file.metadata().map_err(|error| {
        #[cfg(test)]
        super::super::diagnostics::report("readonly.metadata", &error);
        #[cfg(not(test))]
        let _ = error;
        CacheError::Filesystem(FsError::Io)
    })?;
    if metadata.mode() & 0o7777 != mode {
        return Err(CacheError::Filesystem(FsError::UnsafeEntry));
    }
    acl::verify(file).map_err(CacheError::Filesystem)
}

fn open_readonly_artifact(directory: &ArtifactDir, path: &str) -> Result<File, CacheError> {
    check_readonly(&directory.directory, 0o500)?;
    let file = match path.split_once('/') {
        Some((folder, name)) => {
            let child = directory
                .open_subdir(folder)
                .map_err(CacheError::Filesystem)?;
            check_readonly(&child.directory, 0o500)?;
            child.open_file(name).map_err(CacheError::Filesystem)?
        }
        None => directory.open_file(path).map_err(CacheError::Filesystem)?,
    };
    check_readonly(&file, 0o400)?;
    Ok(file)
}

fn create_new(directory: &ArtifactDir, name: &str) -> Result<File, FsError> {
    validate_name(name)?;
    let flags = OFlags::RDWR
        | OFlags::CREATE
        | OFlags::EXCL
        | OFlags::CLOEXEC
        | OFlags::NOFOLLOW
        | OFlags::NONBLOCK;
    let file = File::from(
        rustix::fs::openat(&directory.directory, name, flags, Mode::RUSR | Mode::WUSR)
            .map_err(map_errno)?,
    );
    let metadata = file.metadata().map_err(|_| FsError::Io)?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o7777 != 0o600
    {
        return Err(FsError::UnsafeEntry);
    }
    acl::verify(&file)?;
    Ok(file)
}

fn sync_file(file: &File) -> Result<(), FsError> {
    #[cfg(target_os = "macos")]
    rustix::fs::fcntl_fullfsync(file).map_err(|error| {
        #[cfg(test)]
        super::super::diagnostics::report("artifact.fcntl_fullfsync", &std::io::Error::from(error));
        #[cfg(not(test))]
        let _ = error;
        FsError::Io
    })?;
    #[cfg(target_os = "linux")]
    file.sync_all().map_err(|error| {
        #[cfg(test)]
        super::super::diagnostics::report("artifact.fsync", &error);
        #[cfg(not(test))]
        let _ = error;
        FsError::Io
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Cursor,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::Path,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };

    struct Fixture(tempfile::TempDir);

    impl Fixture {
        fn new() -> Self {
            Self(
                tempfile::Builder::new()
                    .permissions(fs::Permissions::from_mode(0o700))
                    .tempdir_in(
                        std::env::temp_dir()
                            .canonicalize()
                            .expect("physical parent"),
                    )
                    .expect("fixture"),
            )
        }
        fn directory(&self) -> ArtifactDir {
            ArtifactDir::open_root(self.0.path()).expect("root")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fn writable(path: &Path) {
                if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
                    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                        .expect("fixture cleanup only");
                    for entry in fs::read_dir(path).expect("fixture entries") {
                        writable(&entry.expect("fixture entry").path());
                    }
                }
            }
            writable(self.0.path());
        }
    }

    #[test]
    fn stages_a_manifest_bound_buffer_exclusively() {
        let root = Fixture::new();
        let mut stage = ModelArtifactStage::new(
            &root.directory(),
            PinnedMiniLmManifest::synthetic_for_tests(),
        )
        .expect("stage");
        assert!(stage
            .stage_artifact("config.json", Cursor::new(b"abc"))
            .is_ok());
    }

    #[test]
    fn rejects_publication_until_the_exact_manifest_set_is_staged() {
        let root = Fixture::new();
        let mut stage = ModelArtifactStage::new(
            &root.directory(),
            PinnedMiniLmManifest::synthetic_for_tests(),
        )
        .expect("stage");
        stage
            .stage_artifact("config.json", Cursor::new(b"abc"))
            .expect("one artifact");
        assert!(matches!(stage.publish(), Err(CacheError::Incomplete)));
        assert!(fs::read_dir(root.0.path())
            .expect("entries")
            .all(|entry| entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .starts_with('.')));
    }

    #[test]
    fn publishes_only_a_complete_verified_artifact_set_with_its_receipt() {
        let root = Fixture::new();
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let paths: Vec<_> = manifest
            .files()
            .iter()
            .map(|file| file.path().to_owned())
            .collect();
        let mut stage = ModelArtifactStage::new(&root.directory(), manifest).expect("stage");
        for path in paths {
            stage
                .stage_artifact(&path, Cursor::new(b"abc"))
                .expect("artifact");
        }
        let cache = stage.publish();
        assert!(cache.is_ok());
        let cache = cache.expect("complete fixture");
        let expected = receipt_bytes(&cache.manifest).expect("canonical receipt");
        assert_eq!(
            cache.bundle_id(),
            format!("{:x}", Sha256::digest(&expected))
        );
        let path = root.0.path().join(cache.bundle_id());
        assert_eq!(
            fs::read(path.join(RECEIPT_NAME)).expect("receipt"),
            expected
        );
        assert_eq!(path.metadata().expect("root mode").mode() & 0o777, 0o500);
        assert_eq!(
            path.join("1_Pooling")
                .metadata()
                .expect("child mode")
                .mode()
                & 0o777,
            0o500
        );
        for file in cache.manifest.files() {
            assert_eq!(
                path.join(file.path()).metadata().expect("file mode").mode() & 0o777,
                0o400
            );
        }
    }

    /// macOS denies renaming a directory that still holds an open descriptor inside
    /// it, so publication must close every descriptor below the staged directory
    /// before the rename runs.
    #[test]
    fn publication_keeps_no_descriptor_open_inside_the_renamed_directory() {
        let root = Fixture::new();
        let stage = complete_stage(&root);
        let staged = staged_path(&root);
        let pooling = fs::metadata(staged.join("1_Pooling")).expect("pooling metadata");
        let (device, inode) = (pooling.dev(), pooling.ino());
        let open_during_rename = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&open_during_rename);
        let _guard = super::super::super::diagnostics::set_pre_rename_probe(move || {
            if super::super::super::diagnostics::descriptor_open_for(device, inode)
                .expect("conclusive descriptor scan before rename")
            {
                observed.store(true, Ordering::SeqCst);
            }
        });
        stage.publish().expect("publication");
        assert!(
            !open_during_rename.load(Ordering::SeqCst),
            "publication must not rename a directory that keeps descriptors open inside it"
        );
    }

    /// macOS 15 denies renaming a directory whose mode lacks write permission,
    /// so the staged directory must stay writable until the rename completes and
    /// the published directory is frozen afterwards.
    #[test]
    fn publication_keeps_the_staged_directory_writable_until_the_rename() {
        let root = Fixture::new();
        let stage = complete_stage(&root);
        let staged = staged_path(&root);
        let writable_at_rename = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&writable_at_rename);
        let _guard = super::super::super::diagnostics::set_pre_rename_probe(move || {
            let mode = fs::metadata(&staged)
                .map(|metadata| metadata.mode() & 0o777)
                .unwrap_or(0);
            observed.store(mode & 0o200 != 0, Ordering::SeqCst);
        });
        let cache = stage.publish().expect("publication");
        assert!(
            writable_at_rename.load(Ordering::SeqCst),
            "the staged directory must still be writable when it is renamed"
        );
        let published = root.0.path().join(cache.bundle_id());
        assert_eq!(
            published.metadata().expect("published mode").mode() & 0o777,
            0o500,
            "the published directory must be frozen after the rename"
        );
    }

    fn staged_path(root: &Fixture) -> std::path::PathBuf {
        fs::read_dir(root.0.path())
            .expect("fixture entries")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".stage-"))
            })
            .expect("staged directory")
    }

    fn complete_stage(root: &Fixture) -> ModelArtifactStage {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let paths: Vec<_> = manifest
            .files()
            .iter()
            .map(|file| file.path().to_owned())
            .collect();
        let mut stage = ModelArtifactStage::new(&root.directory(), manifest).expect("stage");
        for path in paths {
            stage
                .stage_artifact(&path, Cursor::new(b"abc"))
                .expect("artifact");
        }
        stage
    }

    #[test]
    fn reopens_a_complete_cache_using_its_canonical_receipt() {
        let root = Fixture::new();
        let cache = complete_stage(&root).publish().expect("complete fixture");
        let directory = root
            .directory()
            .open_subdir(cache.bundle_id())
            .expect("published directory");
        let result =
            VerifiedArtifactCache::open(directory, PinnedMiniLmManifest::synthetic_for_tests());
        assert!(result.is_ok());
        assert_eq!(
            result.expect("accepted cache").bundle_id(),
            cache.bundle_id()
        );
    }

    #[test]
    fn reads_a_fresh_verified_buffer_without_exposing_writable_handles() {
        let root = Fixture::new();
        let cache = complete_stage(&root).publish().expect("complete fixture");
        let buffer = cache.read_artifact("1_Pooling/config.json");
        assert!(buffer.is_ok());
        assert_eq!(buffer.expect("verified buffer").into_bytes(), b"abc");
    }

    #[test]
    fn rejects_writable_cache_entries_on_reopen() {
        let root = Fixture::new();
        let cache = complete_stage(&root).publish().expect("complete fixture");
        let path = root.0.path().join(cache.bundle_id()).join("config.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("fixture owner mutation");
        let directory = root
            .directory()
            .open_subdir(cache.bundle_id())
            .expect("published directory");
        assert!(matches!(
            VerifiedArtifactCache::open(directory, PinnedMiniLmManifest::synthetic_for_tests()),
            Err(CacheError::Filesystem(FsError::UnsafeEntry))
        ));
    }

    #[test]
    fn a_file_sync_failure_poisons_the_stage_and_never_completes_the_path() {
        let root = Fixture::new();
        let mut stage = ModelArtifactStage::new(
            &root.directory(),
            PinnedMiniLmManifest::synthetic_for_tests(),
        )
        .expect("stage");
        let result =
            stage.stage_with_sync("config.json", Cursor::new(b"abc"), |_| Err(FsError::Io));
        assert!(matches!(result, Err(CacheError::Filesystem(FsError::Io))));
        assert!(stage.completed.is_empty());
        assert!(matches!(
            stage.stage_artifact("README.md", Cursor::new(b"abc")),
            Err(CacheError::Poisoned)
        ));
        assert!(matches!(stage.publish(), Err(CacheError::Poisoned)));
    }

    struct Unreadable;
    impl Read for Unreadable {
        fn read(&mut self, _bytes: &mut [u8]) -> std::io::Result<usize> {
            panic!("must reject before reading");
        }
    }

    #[test]
    fn invalid_input_can_retry_but_duplicates_and_unlisted_files_do_not_read() {
        let root = Fixture::new();
        let mut stage = ModelArtifactStage::new(
            &root.directory(),
            PinnedMiniLmManifest::synthetic_for_tests(),
        )
        .expect("stage");
        assert!(matches!(
            stage.stage_artifact("config.json", Cursor::new(b"abd")),
            Err(CacheError::Manifest(_))
        ));
        assert!(!root
            .0
            .path()
            .join(&stage.stage.source_name)
            .join("config.json")
            .exists());
        assert!(matches!(
            stage.stage_artifact("../escape", Unreadable),
            Err(CacheError::Manifest(ManifestError::UnknownArtifact))
        ));
        stage
            .stage_artifact("config.json", Cursor::new(b"abc"))
            .expect("valid retry");
        assert!(matches!(
            stage.stage_artifact("config.json", Unreadable),
            Err(CacheError::DuplicateArtifact)
        ));
        assert!(!stage.failed);
    }

    #[test]
    fn preexisting_file_is_preserved_and_poisons_mutating_stage() {
        let root = Fixture::new();
        let mut stage = ModelArtifactStage::new(
            &root.directory(),
            PinnedMiniLmManifest::synthetic_for_tests(),
        )
        .expect("stage");
        let path = root
            .0
            .path()
            .join(&stage.stage.source_name)
            .join("config.json");
        fs::write(&path, b"keep").expect("unexpected fixture file");
        assert!(matches!(
            stage.stage_artifact("config.json", Cursor::new(b"abc")),
            Err(CacheError::Filesystem(FsError::AlreadyExists))
        ));
        assert_eq!(fs::read(path).expect("preserved data"), b"keep");
        assert!(matches!(
            stage.stage_artifact("README.md", Unreadable),
            Err(CacheError::Poisoned)
        ));
        assert!(matches!(stage.publish(), Err(CacheError::Poisoned)));
    }

    #[test]
    fn preexisting_nested_directory_is_not_adopted() {
        let root = Fixture::new();
        let mut stage = ModelArtifactStage::new(
            &root.directory(),
            PinnedMiniLmManifest::synthetic_for_tests(),
        )
        .expect("stage");
        fs::create_dir(
            root.0
                .path()
                .join(&stage.stage.source_name)
                .join("1_Pooling"),
        )
        .expect("unexpected child");
        assert!(matches!(
            stage.stage_artifact("1_Pooling/config.json", Cursor::new(b"abc")),
            Err(CacheError::Filesystem(FsError::AlreadyExists))
        ));
        assert!(matches!(stage.publish(), Err(CacheError::Poisoned)));
    }

    #[test]
    fn publication_reverifies_completed_bytes_and_does_not_trust_the_set() {
        let root = Fixture::new();
        let stage = complete_stage(&root);
        let path = root
            .0
            .path()
            .join(&stage.stage.source_name)
            .join("config.json");
        replace_fixture_bytes(&path, b"abd");
        assert!(matches!(stage.publish(), Err(CacheError::Manifest(_))));
        assert!(fs::read_dir(root.0.path())
            .expect("entries")
            .all(|entry| entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .starts_with('.')));
    }

    fn replace_fixture_bytes(path: &Path, bytes: &[u8]) {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .expect("fixture mutation only");
        fs::write(path, bytes).expect("fixture data");
        fs::set_permissions(path, fs::Permissions::from_mode(0o400))
            .expect("restore readonly fixture");
    }

    #[test]
    fn changed_receipt_bytes_are_rejected_including_truncation_and_extension() {
        let root = Fixture::new();
        let cache = complete_stage(&root).publish().expect("complete fixture");
        let path = root.0.path().join(cache.bundle_id()).join(RECEIPT_NAME);
        let original = fs::read(&path).expect("receipt");
        let mut changed = original.clone();
        changed[0] = b'[';
        let mut extended = original.clone();
        extended.push(b' ');
        for bytes in [changed, original[..original.len() - 1].to_vec(), extended] {
            replace_fixture_bytes(&path, &bytes);
            let directory = root
                .directory()
                .open_subdir(cache.bundle_id())
                .expect("published directory");
            assert!(matches!(
                VerifiedArtifactCache::open(directory, PinnedMiniLmManifest::synthetic_for_tests()),
                Err(CacheError::ReceiptMismatch)
            ));
        }
    }

    #[test]
    fn reopen_rejects_missing_or_corrupt_artifacts_despite_a_correct_receipt() {
        for bytes in [Some(b"abd".as_slice()), Some(b"ab".as_slice()), None] {
            let root = Fixture::new();
            let cache = complete_stage(&root).publish().expect("complete fixture");
            let parent = root.0.path().join(cache.bundle_id());
            let path = parent.join("config.json");
            match bytes {
                Some(bytes) => replace_fixture_bytes(&path, bytes),
                None => {
                    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
                        .expect("fixture root writable");
                    fs::remove_file(path).expect("fixture deletion");
                    fs::set_permissions(&parent, fs::Permissions::from_mode(0o500))
                        .expect("restore fixture");
                }
            }
            let directory = root
                .directory()
                .open_subdir(cache.bundle_id())
                .expect("published directory");
            assert!(VerifiedArtifactCache::open(
                directory,
                PinnedMiniLmManifest::synthetic_for_tests()
            )
            .is_err());
        }
    }

    #[test]
    fn fresh_reads_reject_later_corruption_and_unknown_paths() {
        let root = Fixture::new();
        let cache = complete_stage(&root).publish().expect("complete fixture");
        let buffer = cache
            .read_artifact("config.json")
            .expect("original owned buffer");
        replace_fixture_bytes(
            &root.0.path().join(cache.bundle_id()).join("config.json"),
            b"abd",
        );
        assert_eq!(buffer.as_bytes(), b"abc");
        assert!(matches!(
            cache.read_artifact("config.json"),
            Err(CacheError::Manifest(_))
        ));
        assert!(matches!(
            cache.read_artifact("../escape"),
            Err(CacheError::Manifest(ManifestError::UnknownArtifact))
        ));
    }

    #[test]
    fn preexisting_receipt_and_existing_destination_are_never_replaced() {
        let root = Fixture::new();
        let stage = complete_stage(&root);
        let path = root
            .0
            .path()
            .join(&stage.stage.source_name)
            .join(RECEIPT_NAME);
        fs::write(&path, b"keep").expect("unexpected receipt");
        assert!(matches!(
            stage.publish(),
            Err(CacheError::Filesystem(FsError::AlreadyExists))
        ));
        assert_eq!(fs::read(path).expect("preserved receipt"), b"keep");
        let accepted = complete_stage(&root).publish().expect("first cache");
        let receipt =
            fs::read(root.0.path().join(accepted.bundle_id()).join(RECEIPT_NAME)).expect("receipt");
        assert!(matches!(
            complete_stage(&root).publish(),
            Err(CacheError::Filesystem(FsError::AlreadyExists))
        ));
        assert_eq!(
            fs::read(root.0.path().join(accepted.bundle_id()).join(RECEIPT_NAME))
                .expect("preserved destination"),
            receipt
        );
    }

    #[test]
    fn partial_write_failure_preserves_the_partial_file_and_poisoned_state() {
        let root = Fixture::new();
        let mut stage = ModelArtifactStage::new(
            &root.directory(),
            PinnedMiniLmManifest::synthetic_for_tests(),
        )
        .expect("stage");
        let result = stage.stage_with_io(
            "config.json",
            Cursor::new(b"abc"),
            |file, bytes| {
                file.write_all(&bytes[..2])?;
                Err(std::io::Error::other(
                    "synthetic failure after partial disk write",
                ))
            },
            sync_file,
        );
        assert!(matches!(result, Err(CacheError::Filesystem(FsError::Io))));
        assert_eq!(
            fs::read(
                root.0
                    .path()
                    .join(&stage.stage.source_name)
                    .join("config.json")
            )
            .expect("partial file"),
            b"ab"
        );
        assert!(stage.completed.is_empty());
        assert!(matches!(
            stage.stage_artifact("config.json", Unreadable),
            Err(CacheError::Poisoned)
        ));
        assert!(matches!(stage.publish(), Err(CacheError::Poisoned)));
    }

    /// Reproduces a publication whose freeze failed after the rename: the
    /// final-name entry exists, holds the exact manifest content, and is still
    /// writable because the freeze never completed.
    fn simulate_failed_freeze(root: &Fixture, stage: &ModelArtifactStage) -> String {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let receipt = receipt_bytes(&manifest).expect("receipt");
        let bundle_id = format!("{:x}", Sha256::digest(&receipt));
        let staged = &stage.stage.directory;
        {
            let mut file = create_new(staged, RECEIPT_NAME).expect("receipt file");
            file.write_all(&receipt).expect("receipt bytes");
            file.rewind().expect("receipt rewind");
            file.set_permissions(fs::Permissions::from_mode(0o400))
                .expect("receipt mode");
            file.sync_all().expect("receipt sync");
        }
        let child = staged.open_subdir("1_Pooling").expect("child");
        super::super::super::freeze_directory(&child).expect("child freeze");
        fs::rename(
            root.0.path().join(&stage.stage.source_name),
            root.0.path().join(&bundle_id),
        )
        .expect("publication rename");
        bundle_id
    }

    #[test]
    fn a_failed_freeze_after_the_rename_is_recovered_by_the_next_publication() {
        let root = Fixture::new();
        let stage = complete_stage(&root);
        let bundle_id = simulate_failed_freeze(&root, &stage);
        let target = root.0.path().join(&bundle_id);
        assert!(target.is_dir());
        assert_eq!(
            fs::metadata(&target).expect("target metadata").mode() & 0o777,
            0o700,
            "a failed freeze leaves the published entry writable"
        );
        let cache = complete_stage(&root)
            .publish()
            .expect("recovered publication");
        assert_eq!(cache.bundle_id(), bundle_id);
        assert_eq!(
            fs::metadata(&target).expect("target metadata").mode() & 0o777,
            0o500,
            "the recovered publication must be frozen"
        );
        assert_eq!(
            cache
                .read_artifact("config.json")
                .expect("verified buffer")
                .into_bytes(),
            b"abc"
        );
    }

    /// The publication seam injects a freeze failure after the rename: the
    /// final-name entry exists and is still writable.
    #[test]
    fn an_injected_freeze_failure_leaves_a_writable_entry_that_the_next_publication_recovers() {
        let root = Fixture::new();
        let result = complete_stage(&root).publish_with_freeze(|_| Err(FsError::Io));
        assert!(matches!(
            result,
            Err(CacheError::Filesystem(FsError::OutcomeUnknown))
        ));
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let receipt = receipt_bytes(&manifest).expect("receipt");
        let bundle_id = format!("{:x}", Sha256::digest(&receipt));
        let target = root.0.path().join(&bundle_id);
        assert_eq!(
            fs::metadata(&target).expect("target metadata").mode() & 0o777,
            0o700,
            "the failed freeze left the published entry writable"
        );
        let cache = complete_stage(&root)
            .publish()
            .expect("recovered publication");
        assert_eq!(cache.bundle_id(), bundle_id);
        assert_eq!(
            fs::metadata(&target).expect("target metadata").mode() & 0o777,
            0o500,
            "the recovered publication must be frozen"
        );
    }

    #[test]
    fn a_mismatched_existing_entry_is_never_adopted() {
        let root = Fixture::new();
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let receipt = receipt_bytes(&manifest).expect("receipt");
        let bundle_id = format!("{:x}", Sha256::digest(&receipt));
        let target = root.0.path().join(&bundle_id);
        fs::create_dir(&target).expect("fixture entry");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).expect("entry mode");
        fs::write(target.join(RECEIPT_NAME), b"foreign").expect("foreign receipt");
        fs::set_permissions(target.join(RECEIPT_NAME), fs::Permissions::from_mode(0o400))
            .expect("foreign receipt mode");
        let result = complete_stage(&root).publish();
        assert!(matches!(
            result,
            Err(CacheError::Filesystem(FsError::AlreadyExists))
        ));
        assert_eq!(
            fs::metadata(&target).expect("target metadata").mode() & 0o777,
            0o700,
            "a mismatched entry must stay untouched"
        );
        assert_eq!(
            fs::read(target.join(RECEIPT_NAME)).expect("foreign receipt"),
            b"foreign"
        );
    }

    #[test]
    fn aggregate_limit_is_checked_before_creating_reservations_or_stages() {
        let root = Fixture::new();
        assert!(matches!(
            ModelArtifactStage::new(
                &root.directory(),
                PinnedMiniLmManifest::synthetic_oversized_for_tests()
            ),
            Err(CacheError::TooLarge)
        ));
        assert_eq!(fs::read_dir(root.0.path()).expect("entries").count(), 0);
    }

    #[test]
    fn synthetic_receipt_cannot_authorize_the_real_pinned_manifest() {
        let root = Fixture::new();
        let cache = complete_stage(&root).publish().expect("synthetic fixture");
        let manifest = PinnedMiniLmManifest::load_pinned(Cursor::new(include_bytes!(
            "../../../../dev/muninn/fixtures/artifact-lock.json"
        )))
        .expect("compiled approved lock");
        let directory = root
            .directory()
            .open_subdir(cache.bundle_id())
            .expect("fixture directory");
        assert!(matches!(
            VerifiedArtifactCache::open(directory, manifest),
            Err(CacheError::ReceiptMismatch)
        ));
    }
}
