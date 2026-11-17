//! Descriptor-bound opens with conservative ACL snapshots on supported local filesystems.

use rustix::fs::{Mode, OFlags};
#[path = "artifact_acl.rs"]
mod acl;
#[path = "artifact_once.rs"]
mod once;
#[path = "artifact_recovery.rs"]
pub mod recovery;
#[path = "artifact_staging.rs"]
pub mod staging;
use std::os::unix::fs::MetadataExt;
use std::{
    fs::File,
    path::{Component, Path},
};

#[derive(Debug, PartialEq, Eq)]
pub enum FsError {
    InvalidPath,
    UnsafeEntry,
    NotFound,
    NotRegularFile,
    AclPresent,
    AclUnavailable,
    AlreadyExists,
    OutcomeUnknown,
    Io,
}

/// Descriptor-bound POSIX and native ACL checks, not a permanent permission lease.
/// Cache staging, publication, durability and later privileged mutations are separate.
pub struct ArtifactDir {
    directory: File,
}

impl ArtifactDir {
    /// Recorre una raíz absoluta sin seguir symlinks en ningún componente.
    /// No canonicaliza rutas ni acepta ParentDir; el host conserva su autoridad.
    pub fn open_root(path: &Path) -> Result<Self, FsError> {
        if !path.is_absolute()
            || path
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(FsError::InvalidPath);
        }
        let mut descriptor =
            rustix::fs::open("/", directory_flags(), Mode::empty()).map_err(map_errno)?;
        for part in path.components() {
            if let Component::Normal(name) = part {
                descriptor =
                    rustix::fs::openat(&descriptor, name, directory_flags(), Mode::empty())
                        .map_err(map_errno)?;
            }
        }
        Self::from_directory(File::from(descriptor))
    }

    fn from_directory(directory: File) -> Result<Self, FsError> {
        let metadata = directory.metadata().map_err(|_| FsError::Io)?;
        if !metadata.is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o7077 != 0
            || metadata.mode() & 0o500 != 0o500
        {
            return Err(FsError::UnsafeEntry);
        }
        acl::verify(&directory)?;
        Ok(Self { directory })
    }

    /// Devuelve solo archivos regulares; verificar sus bytes sigue siendo obligatorio.
    /// NONBLOCK evita la espera habitual de una FIFO, no vuelve inocuo un dispositivo.
    pub fn open_file(&self, name: &str) -> Result<File, FsError> {
        validate_name(name)?;
        let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
        let descriptor =
            rustix::fs::openat(&self.directory, name, flags, Mode::empty()).map_err(map_errno)?;
        let file = File::from(descriptor);
        let metadata = file.metadata().map_err(|_| FsError::Io)?;
        if !metadata.is_file() {
            return Err(FsError::NotRegularFile);
        }
        if metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o7022 != 0
            || metadata.mode() & 0o400 == 0
        {
            return Err(FsError::UnsafeEntry);
        }
        acl::verify(&file)?;
        Ok(file)
    }

    /// Cada llamada admite un solo componente y conserva el anclaje por descriptor.
    pub fn open_subdir(&self, name: &str) -> Result<Self, FsError> {
        validate_name(name)?;
        let descriptor =
            rustix::fs::openat(&self.directory, name, directory_flags(), Mode::empty())
                .map_err(map_errno)?;
        Self::from_directory(File::from(descriptor))
    }
}

fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW
}

fn validate_name(name: &str) -> Result<(), FsError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(FsError::InvalidPath);
    }
    Ok(())
}

const fn map_errno(error: rustix::io::Errno) -> FsError {
    match error {
        rustix::io::Errno::EXIST => FsError::AlreadyExists,
        rustix::io::Errno::NOENT => FsError::NotFound,
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => FsError::UnsafeEntry,
        _ => FsError::Io,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};

    fn fixture() -> tempfile::TempDir {
        let parent = std::env::temp_dir()
            .canonicalize()
            .expect("physical temp parent");
        tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(parent)
            .expect("private fixture")
    }

    #[test]
    fn opens_a_private_root() {
        let root = fixture();
        let metadata = root.path().metadata().expect("fixture metadata");
        assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
        assert_eq!(metadata.mode() & 0o777, 0o700, "private fixture contract");
        assert!(ArtifactDir::open_root(root.path()).is_ok());
    }

    fn private_dir(path: &Path) {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .expect("private directory");
    }

    #[cfg(target_os = "macos")]
    fn add_extended_acl(path: &Path) {
        let status = std::process::Command::new("/bin/chmod")
            .args([
                "+a",
                "everyone allow read,readattr,readextattr,readsecurity",
            ])
            .arg(path)
            .status()
            .expect("native fixture ACL tool");
        assert!(status.success());
    }

    #[cfg(target_os = "linux")]
    fn add_extended_acl(path: &Path) {
        let file = File::open(path).expect("fixture descriptor");
        let owner_permissions = if file.metadata().expect("metadata").is_dir() {
            7
        } else {
            6
        };
        rustix::fs::fsetxattr(
            &file,
            "system.posix_acl_access",
            &posix_acl(true, owner_permissions),
            rustix::fs::XattrFlags::empty(),
        )
        .expect("native POSIX ACL fixture");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_a_real_extended_acl_even_with_mode_0700() {
        let root = fixture();
        add_extended_acl(root.path());
        assert_eq!(
            root.path().metadata().expect("metadata").mode() & 0o777,
            0o700
        );
        assert!(matches!(
            ArtifactDir::open_root(root.path()),
            Err(FsError::AclPresent)
        ));
    }

    #[test]
    fn rejects_extended_acls_on_children_and_files() {
        let root = fixture();
        private_dir(&root.path().join("child"));
        let model = root.path().join("model.bin");
        std::fs::write(&model, b"abc").expect("fixture");
        std::fs::set_permissions(&model, std::fs::Permissions::from_mode(0o600)).expect("mode");
        let directory = ArtifactDir::open_root(root.path()).expect("ACL-free root");
        add_extended_acl(&root.path().join("child"));
        add_extended_acl(&model);
        assert_eq!(model.metadata().expect("metadata").mode() & 0o777, 0o600);
        assert!(matches!(
            directory.open_subdir("child"),
            Err(FsError::AclPresent)
        ));
        assert!(matches!(
            directory.open_file("model.bin"),
            Err(FsError::AclPresent)
        ));
    }

    #[test]
    fn acl_queries_follow_the_original_inode_in_both_replacement_directions() {
        let parent = fixture();
        let path = parent.path().join("active");
        private_dir(&path);
        let restricted = File::open(&path).expect("original descriptor");
        add_extended_acl(&path);
        std::fs::rename(&path, parent.path().join("acl-old")).expect("fixture rename");
        private_dir(&path);
        assert!(matches!(acl::verify(&restricted), Err(FsError::AclPresent)));
        let clean = File::open(&path).expect("ACL-free original descriptor");
        std::fs::rename(&path, parent.path().join("clean-old")).expect("fixture rename");
        private_dir(&path);
        add_extended_acl(&path);
        assert!(acl::verify(&clean).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_inherited_extended_acls() {
        let root = fixture();
        let status = std::process::Command::new("/bin/chmod")
            .args(["+a", "everyone allow read,readattr,readextattr,readsecurity,file_inherit,directory_inherit"])
            .arg(root.path()).status().expect("native ACL fixture");
        assert!(status.success());
        private_dir(&root.path().join("child"));
        assert_eq!(
            root.path()
                .join("child")
                .metadata()
                .expect("metadata")
                .mode()
                & 0o777,
            0o700
        );
        assert!(matches!(
            ArtifactDir::open_root(&root.path().join("child")),
            Err(FsError::AclPresent)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_unlinked_or_synthetic_macos_descriptors() {
        let root = fixture();
        let path = root.path().join("model.bin");
        std::fs::write(&path, b"abc").expect("fixture");
        let file = File::open(&path).expect("file");
        std::fs::remove_file(&path).expect("unlink fixture");
        assert!(matches!(acl::verify(&file), Err(FsError::AclUnavailable)));
        let synthetic = File::open("/dev/null").expect("synthetic descriptor");
        assert!(matches!(
            acl::verify(&synthetic),
            Err(FsError::AclUnavailable)
        ));
    }

    #[test]
    fn rejects_a_symlink_in_a_root_ancestor() {
        let parent = fixture();
        private_dir(&parent.path().join("real"));
        private_dir(&parent.path().join("real/child"));
        symlink(parent.path().join("real"), parent.path().join("alias")).expect("fixture symlink");
        assert!(matches!(
            ArtifactDir::open_root(&parent.path().join("alias/child")),
            Err(FsError::UnsafeEntry)
        ));
    }

    #[test]
    fn rejects_broad_or_special_root_mode_bits() {
        let root = fixture();
        for mode in [0o755, 0o2700, 0o1700] {
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(mode))
                .expect("fixture permissions");
            assert!(matches!(
                ArtifactDir::open_root(root.path()),
                Err(FsError::UnsafeEntry)
            ));
        }
    }

    #[test]
    fn rejects_an_artifact_symlink_without_following_it() {
        let root = fixture();
        let outside = fixture();
        std::fs::write(outside.path().join("target"), b"outside").expect("fixture target");
        symlink(outside.path().join("target"), root.path().join("model.bin"))
            .expect("fixture symlink");
        let directory = ArtifactDir::open_root(root.path()).expect("root");
        assert!(matches!(
            directory.open_file("model.bin"),
            Err(FsError::UnsafeEntry)
        ));
    }

    #[test]
    fn opens_a_restricted_child_by_descriptor() {
        let root = fixture();
        private_dir(&root.path().join("1_Pooling"));
        let directory = ArtifactDir::open_root(root.path()).expect("root");
        assert!(directory.open_subdir("1_Pooling").is_ok());
    }

    #[test]
    fn rejects_final_root_links_and_noncanonical_roots() {
        let parent = fixture();
        private_dir(&parent.path().join("real"));
        symlink(parent.path().join("real"), parent.path().join("alias")).expect("symlink");
        assert!(matches!(
            ArtifactDir::open_root(&parent.path().join("alias")),
            Err(FsError::UnsafeEntry)
        ));
        for path in [Path::new("relative"), &parent.path().join("..")] {
            assert!(matches!(
                ArtifactDir::open_root(path),
                Err(FsError::InvalidPath)
            ));
        }
    }

    #[test]
    fn rejects_invalid_child_names_before_io() {
        let root = fixture();
        let directory = ArtifactDir::open_root(root.path()).expect("root");
        for name in [
            "",
            ".",
            "..",
            "../other",
            "/absolute",
            "a/b",
            "a\\b",
            "bad\0name",
            "é",
        ] {
            assert!(matches!(
                directory.open_file(name),
                Err(FsError::InvalidPath)
            ));
            assert!(matches!(
                directory.open_subdir(name),
                Err(FsError::InvalidPath)
            ));
        }
        assert!(matches!(
            directory.open_file(&"a".repeat(129)),
            Err(FsError::InvalidPath)
        ));
        std::fs::write(root.path().join("a".repeat(128)), b"abc").expect("long valid basename");
        assert!(directory.open_file(&"a".repeat(128)).is_ok());
    }

    #[test]
    fn rejects_linked_children_nonregular_files_and_missing_entries() {
        let root = fixture();
        private_dir(&root.path().join("child"));
        symlink(root.path().join("child"), root.path().join("alias")).expect("symlink");
        symlink(root.path().join("missing"), root.path().join("dangling"))
            .expect("dangling symlink");
        let directory = ArtifactDir::open_root(root.path()).expect("root");
        assert!(matches!(
            directory.open_subdir("alias"),
            Err(FsError::UnsafeEntry)
        ));
        assert!(matches!(
            directory.open_file("dangling"),
            Err(FsError::UnsafeEntry)
        ));
        assert!(matches!(
            directory.open_file("child"),
            Err(FsError::NotRegularFile)
        ));
        assert!(matches!(
            directory.open_file("absent"),
            Err(FsError::NotFound)
        ));
    }

    #[test]
    fn checks_file_mode_and_accepts_readonly_owner_files() {
        let root = fixture();
        let path = root.path().join("model.bin");
        std::fs::write(&path, b"abc").expect("fixture");
        let directory = ArtifactDir::open_root(root.path()).expect("root");
        for mode in [0o666, 0o4700] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("mode");
            assert!(matches!(
                directory.open_file("model.bin"),
                Err(FsError::UnsafeEntry)
            ));
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).expect("readonly");
        assert!(directory.open_file("model.bin").is_ok());
    }

    #[test]
    fn checks_child_modes_and_accepts_readonly_owner_directories() {
        let root = fixture();
        let child = root.path().join("child");
        private_dir(&child);
        let directory = ArtifactDir::open_root(root.path()).expect("root");
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o755))
            .expect("broad mode");
        assert!(matches!(
            directory.open_subdir("child"),
            Err(FsError::UnsafeEntry)
        ));
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o500))
            .expect("readonly child");
        assert!(directory.open_subdir("child").is_ok());
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o500))
            .expect("readonly root");
        let result = ArtifactDir::open_root(root.path()).is_ok();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restore fixture cleanup");
        assert!(result);
    }

    #[test]
    fn remains_anchored_after_root_path_replacement_and_verifies_the_buffer() {
        let parent = fixture();
        let path = parent.path().join("active");
        private_dir(&path);
        std::fs::write(path.join("model.bin"), b"abc").expect("original");
        let directory = ArtifactDir::open_root(&path).expect("root");
        std::fs::rename(&path, parent.path().join("moved"))
            .expect("fixture rename, not cache promotion");
        private_dir(&path);
        std::fs::write(path.join("model.bin"), b"xyz").expect("replacement");
        let file = directory.open_file("model.bin").expect("anchored file");
        let verified = crate::artifacts::load_verified(
            file,
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .expect("verified original");
        assert_eq!(verified.as_bytes(), b"abc");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_a_fifo_without_waiting_for_a_writer() {
        let root = fixture();
        let directory = ArtifactDir::open_root(root.path()).expect("root");
        rustix::fs::mkfifoat(&directory.directory, "fifo", Mode::RUSR | Mode::WUSR)
            .expect("fixture FIFO");
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender
                .send(directory.open_file("fifo").err())
                .expect("result channel");
        });
        let result = receiver.recv_timeout(std::time::Duration::from_secs(5));
        if result.is_err() {
            let _release = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(root.path().join("fifo"))
                .expect("release blocked FIFO reader");
            worker.join().expect("worker");
            panic!("opening a FIFO waited for a writer");
        }
        worker.join().expect("worker");
        assert_eq!(
            result.expect("completed without writer"),
            Some(FsError::NotRegularFile)
        );
    }

    #[cfg(target_os = "linux")]
    fn posix_acl(named_user: bool, owner_permissions: u16) -> Vec<u8> {
        let undefined = u32::MAX;
        let entries = if named_user {
            vec![
                (1_u16, owner_permissions, undefined),
                (2, 5, 65534),
                (4, 0, undefined),
                (16, 0, undefined),
                (32, 0, undefined),
            ]
        } else {
            vec![
                (1_u16, owner_permissions, undefined),
                (4, 0, undefined),
                (32, 0, undefined),
            ]
        };
        let mut bytes = 2_u32.to_le_bytes().to_vec();
        for (tag, permissions, id) in entries {
            bytes.extend(tag.to_le_bytes());
            bytes.extend(permissions.to_le_bytes());
            bytes.extend(id.to_le_bytes());
        }
        bytes
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_posix_access_and_default_acls_even_with_mode_0700() {
        for (name, named_user) in [
            ("system.posix_acl_access", true),
            ("system.posix_acl_default", false),
        ] {
            let root = fixture();
            let file = File::open(root.path()).expect("fixture descriptor");
            rustix::fs::fsetxattr(
                &file,
                name,
                &posix_acl(named_user, 7),
                rustix::fs::XattrFlags::empty(),
            )
            .expect("native POSIX ACL");
            assert_eq!(file.metadata().expect("metadata").mode() & 0o777, 0o700);
            assert!(matches!(
                ArtifactDir::open_root(root.path()),
                Err(FsError::AclPresent)
            ));
        }
    }
}
