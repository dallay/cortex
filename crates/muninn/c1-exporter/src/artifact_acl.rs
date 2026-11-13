//! Native descriptor checks. No pathname fallback or ACL modification.

use super::FsError;
use std::fs::File;

#[cfg(target_os = "macos")]
pub(super) fn verify(file: &File) -> Result<(), FsError> {
    use std::{
        ffi::CStr,
        os::{fd::AsFd, unix::fs::MetadataExt},
    };
    let before = file.metadata().map_err(|_| FsError::AclUnavailable)?;
    let filesystem = rustix::fs::fstatfs(file).map_err(|_| FsError::AclUnavailable)?;
    let name = filesystem.f_fstypename.map(|byte| byte.to_ne_bytes()[0]);
    if before.nlink() == 0
        || CStr::from_bytes_until_nul(&name)
            .map_err(|_| FsError::AclUnavailable)?
            .to_bytes()
            != b"apfs"
    {
        return Err(FsError::AclUnavailable);
    }
    // The pinned wrapper maps native ENOENT to empty. Restrict that assumption
    // to linked APFS vnodes; do not generalize it to network/synthetic filesystems.
    let result = calcifer_macos_acl::read_acl(file.as_fd());
    let after = file.metadata().map_err(|_| FsError::AclUnavailable)?;
    if after.nlink() == 0 || before.dev() != after.dev() || before.ino() != after.ino() {
        return Err(FsError::AclUnavailable);
    }
    classify_native(result)
}

#[cfg(target_os = "macos")]
fn classify_native(result: std::io::Result<calcifer_macos_acl::Acl>) -> Result<(), FsError> {
    let acl = result.map_err(|_| FsError::AclUnavailable)?;
    if acl.is_empty() {
        Ok(())
    } else {
        Err(FsError::AclPresent)
    }
}

#[cfg(target_os = "linux")]
pub(super) fn verify(file: &File) -> Result<(), FsError> {
    let filesystem = rustix::fs::fstatfs(file).map_err(|_| FsError::AclUnavailable)?;
    // Linux uapi magic.h: ext-family, tmpfs and overlay. Do not assume that
    // absent POSIX attributes describe network/synthetic filesystem ACLs.
    if !matches!(filesystem.f_type, 0xef53 | 0x0102_1994 | 0x794c_7630) {
        return Err(FsError::AclUnavailable);
    }
    classify_xattr(rustix::fs::fgetxattr_query(file, "system.posix_acl_access"))?;
    if file
        .metadata()
        .map_err(|_| FsError::AclUnavailable)?
        .is_dir()
    {
        classify_xattr(rustix::fs::fgetxattr_query(
            file,
            "system.posix_acl_default",
        ))?;
    }
    Ok(())
}

#[cfg(any(test, target_os = "linux"))]
const fn classify_xattr(result: Result<usize, rustix::io::Errno>) -> Result<(), FsError> {
    match result {
        Err(rustix::io::Errno::NODATA) => Ok(()),
        Ok(_) => Err(FsError::AclPresent),
        Err(_) => Err(FsError::AclUnavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_missing_posix_acl_attributes_count_as_absence() {
        assert_eq!(classify_xattr(Err(rustix::io::Errno::NODATA)), Ok(()));
        for result in [Ok(0), Ok(24)] {
            assert_eq!(classify_xattr(result), Err(FsError::AclPresent));
        }
        for error in [
            rustix::io::Errno::PERM,
            rustix::io::Errno::OPNOTSUPP,
            rustix::io::Errno::IO,
        ] {
            assert_eq!(classify_xattr(Err(error)), Err(FsError::AclUnavailable));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_errors_unknown_flags_and_entries_never_count_as_empty() {
        use calcifer_macos_acl::{Acl, Entry};
        assert_eq!(classify_native(Ok(Acl::default())), Ok(()));
        assert_eq!(
            classify_native(Ok(Acl {
                flags: 1 << 31,
                entries: Vec::new()
            })),
            Err(FsError::AclPresent)
        );
        assert_eq!(
            classify_native(Ok(Acl {
                flags: 0,
                entries: vec![Entry {
                    tag: 99,
                    flags: u32::MAX,
                    permissions: u32::MAX
                }]
            })),
            Err(FsError::AclPresent)
        );
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::InvalidData,
            std::io::ErrorKind::NotFound,
        ] {
            assert_eq!(
                classify_native(Err(std::io::Error::from(kind))),
                Err(FsError::AclUnavailable)
            );
        }
    }
}
