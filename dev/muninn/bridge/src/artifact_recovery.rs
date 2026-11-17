//! Exact recovery of a known stopped operator; not a marker-based liveness detector.

use super::staging::model::{ModelArtifactStage, VerifiedArtifactCache};
use super::{ArtifactDir, FsError};
use crate::{
    artifact_fs::staging::model::CacheError,
    model_manifest::{ManifestError, PinnedMiniLmManifest},
};
use rustix::fs::{Mode, OFlags};
use std::io::{Read, Seek};
use std::{fs::File, os::unix::fs::MetadataExt};
use uuid::Uuid;
const RECOVERY_MARKER: &str = ".muninn-recovery-once";
#[path = "artifact_recovery_snapshot.rs"]
mod snapshot;

#[derive(Clone, Copy)]
pub struct RecoveryRequest {
    pub root_device: u64,
    pub root_inode: u64,
    pub source_id: Uuid,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryError {
    Filesystem(FsError),
    Manifest(ManifestError),
    Cache(CacheError),
    ChangedState,
    Budget,
}

/// Read-only integrity snapshot, not authorization or evidence of worker termination.
pub struct RecoverySnapshot {
    parent: ArtifactDir,
    source: ArtifactDir,
    manifest: PinnedMiniLmManifest,
    request: RecoveryRequest,
    paths: Vec<String>,
    old_bytes: u64,
    control_ids: Vec<(u64, u64)>,
    file_ids: Vec<(u64, u64)>,
}

pub struct RecoveryClaim {
    snapshot: RecoverySnapshot,
    marker: File,
}

pub struct RecoveryRun {
    stage: ModelArtifactStage,
    paths: Vec<String>,
    _marker: File,
}

impl RecoverySnapshot {
    /// The trusted operator must already have observed the legacy worker stop.
    pub fn inspect_known_stopped(
        parent: &ArtifactDir,
        manifest: PinnedMiniLmManifest,
        request: RecoveryRequest,
    ) -> Result<Self, RecoveryError> {
        let parent = ArtifactDir::from_directory(
            parent
                .directory
                .try_clone()
                .map_err(|_| RecoveryError::Filesystem(FsError::Io))?,
        )
        .map_err(RecoveryError::Filesystem)?;
        let checked = snapshot::inspect(&parent, &manifest, request, false)?;
        Ok(Self {
            parent,
            source: checked.source,
            paths: checked.paths,
            old_bytes: checked.old_bytes,
            control_ids: checked.control_ids,
            file_ids: checked.file_ids,
            manifest,
            request,
        })
    }

    pub fn claim(self) -> Result<RecoveryClaim, RecoveryError> {
        self.claim_with_sync(File::sync_all)
    }

    fn claim_with_sync<S: FnMut(&File) -> std::io::Result<()>>(
        self,
        mut sync: S,
    ) -> Result<RecoveryClaim, RecoveryError> {
        self.revalidate(false)?;
        self.check_space()?;
        let flags = OFlags::RDWR
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK;
        let marker = File::from(
            rustix::fs::openat(
                &self.parent.directory,
                RECOVERY_MARKER,
                flags,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|error| RecoveryError::Filesystem(super::map_errno(error)))?,
        );
        snapshot::check_file(&marker, 0o600, self.request.root_device)?;
        sync(&marker).map_err(|_| RecoveryError::Filesystem(FsError::OutcomeUnknown))?;
        sync(&self.parent.directory)
            .map_err(|_| RecoveryError::Filesystem(FsError::OutcomeUnknown))?;
        self.revalidate(true)?;
        check_marker(&self.parent, &marker, self.request.root_device)?;
        Ok(RecoveryClaim {
            snapshot: self,
            marker,
        })
    }

    fn revalidate(&self, claimed: bool) -> Result<(), RecoveryError> {
        let current = snapshot::inspect(&self.parent, &self.manifest, self.request, claimed)?;
        if identity(&current.source.directory)? != identity(&self.source.directory)?
            || current.paths != self.paths
            || current.old_bytes != self.old_bytes
            || current.control_ids != self.control_ids
            || current.file_ids != self.file_ids
        {
            return Err(RecoveryError::ChangedState);
        }
        Ok(())
    }

    fn check_space(&self) -> Result<(), RecoveryError> {
        let total = self
            .manifest
            .files()
            .iter()
            .try_fold(0_u64, |sum, file| sum.checked_add(file.size()))
            .ok_or(RecoveryError::Budget)?;
        let needed = snapshot::budget(self.old_bytes, total)?;
        let space = rustix::fs::fstatvfs(&self.parent.directory)
            .map_err(|_| RecoveryError::Filesystem(FsError::Io))?;
        if u128::from(space.f_bavail) * u128::from(space.f_frsize) < u128::from(needed) {
            return Err(RecoveryError::Budget);
        }
        Ok(())
    }
}

fn identity(file: &File) -> Result<(u64, u64), RecoveryError> {
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::Filesystem(FsError::Io))?;
    Ok((metadata.dev(), metadata.ino()))
}

fn check_marker(parent: &ArtifactDir, marker: &File, device: u64) -> Result<(), RecoveryError> {
    let named = parent
        .open_file(RECOVERY_MARKER)
        .map_err(RecoveryError::Filesystem)?;
    snapshot::check_file(&named, 0o600, device)?;
    if identity(&named)? != identity(marker)?
        || named
            .metadata()
            .map_err(|_| RecoveryError::Filesystem(FsError::Io))?
            .len()
            != 0
    {
        return Err(RecoveryError::ChangedState);
    }
    Ok(())
}

impl RecoveryClaim {
    /// Requires a trusted host observation that the legacy worker has stopped.
    pub fn claim_known_stopped(
        parent: &ArtifactDir,
        manifest: PinnedMiniLmManifest,
        request: RecoveryRequest,
    ) -> Result<Self, RecoveryError> {
        RecoverySnapshot::inspect_known_stopped(parent, manifest, request)?.claim()
    }

    pub fn begin(self) -> Result<RecoveryRun, RecoveryError> {
        self.begin_with_copy(|stage, path, file| {
            stage
                .stage_artifact(path, file)
                .map_err(RecoveryError::Cache)
        })
    }

    fn begin_with_copy<C>(self, mut copy: C) -> Result<RecoveryRun, RecoveryError>
    where
        C: FnMut(&mut ModelArtifactStage, &str, File) -> Result<(), RecoveryError>,
    {
        self.snapshot.revalidate(true)?;
        self.snapshot.check_space()?;
        check_marker(
            &self.snapshot.parent,
            &self.marker,
            self.snapshot.request.root_device,
        )?;
        let mut stage = ModelArtifactStage::new(&self.snapshot.parent, self.snapshot.manifest)
            .map_err(RecoveryError::Cache)?;
        for (index, path) in self.snapshot.paths.iter().enumerate() {
            let mut file = self
                .snapshot
                .source
                .open_file(path)
                .map_err(RecoveryError::Filesystem)?;
            snapshot::check_file(&file, 0o400, self.snapshot.request.root_device)?;
            if identity(&file)? != self.snapshot.file_ids[index] {
                return Err(RecoveryError::ChangedState);
            }
            file.rewind()
                .map_err(|_| RecoveryError::Filesystem(FsError::Io))?;
            copy(&mut stage, path, file)?;
        }
        Ok(RecoveryRun {
            stage,
            paths: self.snapshot.paths,
            _marker: self.marker,
        })
    }
}

impl RecoveryRun {
    pub fn reused_paths(&self) -> &[String] {
        &self.paths
    }

    pub fn stage_artifact<R: Read>(&mut self, path: &str, reader: R) -> Result<(), RecoveryError> {
        self.stage
            .stage_artifact(path, reader)
            .map_err(RecoveryError::Cache)
    }

    pub fn publish(self) -> Result<VerifiedArtifactCache, RecoveryError> {
        self.stage.publish().map_err(RecoveryError::Cache)
    }
}

#[cfg(test)]
#[path = "artifact_recovery_tests.rs"]
mod tests;
