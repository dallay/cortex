//! Lectura única y acotada de restricciones adicionales del proyecto.
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use huginn_core::{AgentError, Result};
use huginn_guards::Guardrails;
use std::{io::Read, path::Path, sync::Arc};

fn invalid() -> AgentError {
    AgentError::Composition("guardrails: cannot load project restrictions safely".into())
}

pub fn load_guardrails(workspace: &Path) -> Result<Arc<Guardrails>> {
    let root = cap_std::fs::Dir::open_ambient_dir(workspace, cap_std::ambient_authority())?;
    let huginn = match root.open_dir_nofollow(".huginn") {
        Ok(dir) => dir,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Arc::default()),
        Err(_) => return Err(invalid()),
    };
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = match huginn.open_with("guardrails.toml", &options) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Arc::default()),
        Err(_) => return Err(invalid()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 65_536 {
        return Err(invalid());
    }
    let mut text = String::new();
    file.take(65_537)
        .read_to_string(&mut text)
        .map_err(|_| invalid())?;
    Guardrails::from_toml(&text)
        .map(Arc::new)
        .map_err(|_| invalid())
}
