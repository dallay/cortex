use super::{engine::error, MAX_SOURCE_BYTES};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use huginn_core::Result;
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};

pub(super) struct Workspace {
    path: PathBuf,
    directory: Dir,
}

impl Workspace {
    pub fn pin(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            directory: Dir::open_ambient_dir(path, ambient_authority())?,
        })
    }

    pub fn check_context(&self, path: &Path) -> Result<()> {
        if path != self.path {
            return Err(error("tool context does not match the selected workspace"));
        }
        Ok(())
    }
}

pub(super) fn validate(
    workspace: &Path,
    path: &Path,
    guards: &huginn_guards::Guardrails,
) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.as_os_str().as_encoded_bytes().len() > 4096
        || path
            .to_str()
            .is_none_or(|text| text.chars().any(char::is_control))
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(error("path must be workspace-relative without traversal"));
    }
    guards.check_path(workspace, path).map_err(error)
}

pub(super) fn read(
    workspace: &Workspace,
    path: &Path,
    guards: &huginn_guards::Guardrails,
) -> Result<Vec<u8>> {
    validate(&workspace.path, path, guards)?;
    // Resolve each component relative to an already-open capability. Reject
    // symlinks (including internal aliases); never canonicalize then read.
    let mut directory = workspace.directory.try_clone()?;
    let mut parts = path.components().peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_some() {
            directory = directory.open_dir_nofollow(part.as_os_str())?;
            continue;
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = directory.open_with(part.as_os_str(), &options)?;
        if !file.metadata()?.is_file() {
            return Err(error("snapshot requires a regular file"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_SOURCE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(error("source exceeds 1 MiB"));
        }
        return Ok(bytes);
    }
    Err(error("missing snapshot file"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn pinned_workspace_rejects_concurrent_child_directory_redirection() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        let parked = root.path().join("parked");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("sample.rs"), "authorized").unwrap();
        std::fs::write(outside.path().join("sample.rs"), "secret").unwrap();
        let workspace = Workspace::pin(root.path()).unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                for _ in 0..100 {
                    std::fs::rename(&child, &parked).unwrap();
                    std::os::unix::fs::symlink(outside.path(), &child).unwrap();
                    std::thread::yield_now();
                    std::fs::remove_file(&child).unwrap();
                    std::fs::rename(&parked, &child).unwrap();
                }
            });
            barrier.wait();
            for _ in 0..100 {
                if let Ok(bytes) = read(
                    &workspace,
                    Path::new("child/sample.rs"),
                    &huginn_guards::Guardrails::default(),
                ) {
                    assert_eq!(bytes, b"authorized");
                }
            }
        });
        assert_eq!(
            read(
                &workspace,
                Path::new("child/sample.rs"),
                &huginn_guards::Guardrails::default()
            )
            .unwrap(),
            b"authorized"
        );
    }
}
