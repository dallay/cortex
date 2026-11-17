//! Exact bounded namespace and pinned-byte checks. No pathname reopening or effects.

use super::{ArtifactDir, FsError, RecoveryError, RecoveryRequest};
use crate::model_manifest::PinnedMiniLmManifest;
use rustix::fs::Dir;
use std::{collections::BTreeSet, fs::File, io::Seek, os::unix::fs::MetadataExt};

pub(super) struct Checked {
    pub source: ArtifactDir,
    pub paths: Vec<String>,
    pub old_bytes: u64,
    pub control_ids: Vec<(u64, u64)>,
    pub file_ids: Vec<(u64, u64)>,
}

pub(super) fn inspect(
    parent: &ArtifactDir,
    manifest: &PinnedMiniLmManifest,
    request: RecoveryRequest,
    claimed: bool,
) -> Result<Checked, RecoveryError> {
    let checked_parent = ArtifactDir::from_directory(
        parent
            .directory
            .try_clone()
            .map_err(|_| RecoveryError::Filesystem(FsError::Io))?,
    )
    .map_err(RecoveryError::Filesystem)?;
    let metadata = checked_parent
        .directory
        .metadata()
        .map_err(|_| RecoveryError::Filesystem(FsError::Io))?;
    if metadata.mode() & 0o7777 != 0o700
        || metadata.dev() != request.root_device
        || metadata.ino() != request.root_inode
    {
        return Err(RecoveryError::ChangedState);
    }
    let source_name = format!(".stage-{}", request.source_id);
    let lease_name = format!(".lease-{}", request.source_id);
    let mut expected = vec![
        ".muninn-prepare-once".to_owned(),
        lease_name.clone(),
        source_name.clone(),
    ];
    if claimed {
        expected.push(super::RECOVERY_MARKER.to_owned());
    }
    exact_names(&checked_parent, &expected)?;
    let mut control_ids = Vec::new();
    for name in [".muninn-prepare-once", &lease_name] {
        let file = checked_parent
            .open_file(name)
            .map_err(RecoveryError::Filesystem)?;
        check_file(&file, 0o600, request.root_device)?;
        if file
            .metadata()
            .map_err(|_| RecoveryError::Filesystem(FsError::Io))?
            .len()
            != 0
        {
            return Err(RecoveryError::ChangedState);
        }
        let metadata = file
            .metadata()
            .map_err(|_| RecoveryError::Filesystem(FsError::Io))?;
        control_ids.push((metadata.dev(), metadata.ino()));
    }
    let source = checked_parent
        .open_subdir(&source_name)
        .map_err(RecoveryError::Filesystem)?;
    let source_metadata = source
        .directory
        .metadata()
        .map_err(|_| RecoveryError::Filesystem(FsError::Io))?;
    if source_metadata.mode() & 0o7777 != 0o700 || source_metadata.dev() != request.root_device {
        return Err(RecoveryError::ChangedState);
    }
    let paths: Vec<_> = manifest
        .files()
        .iter()
        .take(3)
        .map(|file| file.path().to_owned())
        .collect();
    if paths.len() != 3
        || paths
            .iter()
            .any(|path| super::super::validate_name(path).is_err())
    {
        return Err(RecoveryError::ChangedState);
    }
    exact_names(&source, &paths)?;
    let mut old_bytes = 0_u64;
    let mut file_ids = Vec::new();
    for path in &paths {
        let mut file = source.open_file(path).map_err(RecoveryError::Filesystem)?;
        check_file(&file, 0o400, request.root_device)?;
        let metadata = file
            .metadata()
            .map_err(|_| RecoveryError::Filesystem(FsError::Io))?;
        file_ids.push((metadata.dev(), metadata.ino()));
        file.rewind()
            .map_err(|_| RecoveryError::Filesystem(FsError::Io))?;
        let bytes = manifest
            .verify_artifact(path, file)
            .map_err(RecoveryError::Manifest)?;
        old_bytes = old_bytes
            .checked_add(u64::try_from(bytes.as_bytes().len()).map_err(|_| RecoveryError::Budget)?)
            .ok_or(RecoveryError::Budget)?;
    }
    Ok(Checked {
        source,
        paths,
        old_bytes,
        control_ids,
        file_ids,
    })
}

pub(super) fn check_file(file: &File, mode: u32, device: u64) -> Result<(), RecoveryError> {
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::Filesystem(FsError::Io))?;
    if !metadata.is_file()
        || metadata.mode() & 0o7777 != mode
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.nlink() != 1
        || metadata.dev() != device
    {
        return Err(RecoveryError::ChangedState);
    }
    super::super::acl::verify(file).map_err(RecoveryError::Filesystem)
}

fn exact_names(directory: &ArtifactDir, expected: &[String]) -> Result<(), RecoveryError> {
    let expected: BTreeSet<_> = expected.iter().cloned().collect();
    let mut actual = BTreeSet::new();
    let mut entries = Dir::read_from(&directory.directory)
        .map_err(|error| RecoveryError::Filesystem(super::super::map_errno(error)))?;
    for _ in 0..expected.len() + 3 {
        let Some(entry) = entries.read() else {
            return if actual == expected {
                Ok(())
            } else {
                Err(RecoveryError::ChangedState)
            };
        };
        let entry =
            entry.map_err(|error| RecoveryError::Filesystem(super::super::map_errno(error)))?;
        let name = entry
            .file_name()
            .to_str()
            .map_err(|_| RecoveryError::ChangedState)?;
        if matches!(name, "." | "..") {
            continue;
        }
        if name.len() > 128 || !expected.contains(name) || !actual.insert(name.to_owned()) {
            return Err(RecoveryError::ChangedState);
        }
    }
    Err(RecoveryError::ChangedState)
}

pub(super) fn budget(old_bytes: u64, new_bytes: u64) -> Result<u64, RecoveryError> {
    let needed = new_bytes
        .checked_add(64 * 1024)
        .ok_or(RecoveryError::Budget)?;
    let projected = old_bytes.checked_add(needed).ok_or(RecoveryError::Budget)?;
    if old_bytes == 0 || new_bytes == 0 || projected > crate::artifacts::MAX_ARTIFACT_BYTES {
        return Err(RecoveryError::Budget);
    }
    Ok(needed)
}
