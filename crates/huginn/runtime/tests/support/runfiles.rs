use std::path::{Path, PathBuf};

pub fn resolve(runfiles_relative: &str, cargo_path: &Path) -> PathBuf {
    resolve_from(
        std::env::var_os("RUNFILES_DIR").as_deref(),
        runfiles_relative,
        cargo_path,
    )
}

fn resolve_from(
    runfiles_dir: Option<&std::ffi::OsStr>,
    runfiles_relative: &str,
    cargo_path: &Path,
) -> PathBuf {
    match runfiles_dir {
        Some(dir) => PathBuf::from(dir).join(runfiles_relative),
        None => cargo_path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_from;
    use std::{ffi::OsStr, path::Path};

    #[test]
    fn uses_runfiles_directory_when_available() {
        assert_eq!(
            resolve_from(
                Some(OsStr::new("/bazel/runfiles")),
                "repo/file.py",
                Path::new("/cargo/file.py")
            ),
            Path::new("/bazel/runfiles/repo/file.py")
        );
    }

    #[test]
    fn falls_back_to_cargo_path_without_runfiles_directory() {
        assert_eq!(
            resolve_from(None, "repo/file.py", Path::new("/cargo/file.py")),
            Path::new("/cargo/file.py")
        );
    }
}
