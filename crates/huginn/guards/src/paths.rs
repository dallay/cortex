use std::path::{Component, Path, PathBuf};

pub fn normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            _ => result.push(component),
        }
    }
    result
}

pub fn sensitive(path: &Path) -> bool {
    path.components().any(|part| {
        let Component::Normal(value) = part else {
            return false;
        };
        let Some(name) = value.to_str() else {
            return true;
        };
        let name = name.to_ascii_lowercase();
        name == ".env"
            || name.starts_with(".env.")
            || matches!(
                name.as_str(),
                ".ssh"
                    | ".gnupg"
                    | "id_rsa"
                    | "id_dsa"
                    | "id_ecdsa"
                    | "id_ed25519"
                    | "credentials.json"
                    | "credentials"
                    | "guardrails.toml"
            )
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || name.ends_with(".p12")
            || name.ends_with(".pfx")
    })
}

pub fn starts_with_ascii_case_insensitive(path: &Path, prefix: &Path) -> bool {
    let mut components = path.components();
    prefix.components().all(|expected| {
        components.next().is_some_and(|actual| {
            actual
                .as_os_str()
                .as_encoded_bytes()
                .eq_ignore_ascii_case(expected.as_os_str().as_encoded_bytes())
        })
    })
}

/// Resuelve también el padre de un destino nuevo. Los enlaces colgantes fallan.
pub fn resolve_nearest(path: &Path) -> Result<PathBuf, &'static str> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                for part in missing.into_iter().rev() {
                    resolved.push(part);
                }
                return Ok(normalized(&resolved));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if ancestor
                    .symlink_metadata()
                    .is_ok_and(|m| m.file_type().is_symlink())
                {
                    return Err("guardrails: dangling path alias");
                }
                missing.push(ancestor.file_name().ok_or("guardrails: invalid path")?);
                ancestor = ancestor
                    .parent()
                    .ok_or("guardrails: path resolution failed")?;
            }
            Err(_) => return Err("guardrails: path resolution failed"),
        }
    }
}
