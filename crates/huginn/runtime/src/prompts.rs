//! Scoped prompt files; every invocation re-reads and revalidates the source.
use huginn_application::prompts::{
    self, PromptCatalog, PromptEntry, PromptTemplates, MAX_TEMPLATES, MAX_TEMPLATE_BYTES,
};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub struct FilesystemPrompts {
    project: PathBuf,
    home: Option<PathBuf>,
    project_dir: cap_std::fs::Dir,
    home_dir: Option<cap_std::fs::Dir>,
}
impl FilesystemPrompts {
    pub fn new(workspace: &Path, home: Option<&Path>) -> Result<Self, String> {
        let workspace = workspace.canonicalize().map_err(|e| e.to_string())?;
        let project = workspace
            .ancestors()
            .find(|p| p.join(".git").exists())
            .unwrap_or(&workspace)
            .to_path_buf();
        let home = home
            .map(|p| p.canonicalize().map_err(|e| e.to_string()))
            .transpose()?;
        let project_dir =
            cap_std::fs::Dir::open_ambient_dir(&project, cap_std::ambient_authority())
                .map_err(|e| e.to_string())?;
        let home_dir = home
            .as_ref()
            .map(|p| {
                cap_std::fs::Dir::open_ambient_dir(p, cap_std::ambient_authority())
                    .map_err(|e| e.to_string())
            })
            .transpose()?;
        Ok(Self {
            project,
            home,
            project_dir,
            home_dir,
        })
    }
    fn roots(&self) -> Vec<(PathBuf, bool)> {
        let mut roots = Vec::new();
        if let Some(home) = &self.home {
            roots.push((home.clone(), false));
        }
        roots.push((self.project.clone(), true));
        roots
    }
    fn read(&self, scope: &Path, name: &str) -> Result<String, String> {
        let root = prompts::project_prompt_root(scope);
        // Reject relocated roots as well as entries. Scope ancestors are canonical.
        let canonical = root.canonicalize().map_err(|e| e.to_string())?;
        if canonical != root {
            return Err("El directorio de plantillas no puede ser un symlink".into());
        }
        let path = root.join(format!("{name}.md"));
        let before = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !before.is_file()
            || before.file_type().is_symlink()
            || path.canonicalize().map_err(|e| e.to_string())? != path
        {
            return Err("La plantilla debe ser un archivo regular dentro de su scope".into());
        }
        use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
        let scope_dir = if scope == self.project {
            &self.project_dir
        } else {
            self.home_dir
                .as_ref()
                .ok_or("Directorio personal no disponible")?
        };
        // Descriptor-relative opens pin every directory; no ambient path race can
        // redirect a file read to another scope, including on Windows.
        let huginn = scope_dir
            .open_dir_nofollow(".huginn")
            .map_err(|e| e.to_string())?;
        let directory = huginn
            .open_dir_nofollow("prompts")
            .map_err(|e| e.to_string())?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = directory
            .open_with(format!("{name}.md"), &options)
            .map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_TEMPLATE_BYTES as u64 {
            return Err("Plantilla no regular o demasiado grande".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_TEMPLATE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_TEMPLATE_BYTES {
            return Err("Plantilla demasiado grande".into());
        }
        if root.canonicalize().map_err(|e| e.to_string())? != root
            || path.canonicalize().map_err(|e| e.to_string())? != path
        {
            return Err("El scope cambió durante la lectura".into());
        }
        String::from_utf8(bytes).map_err(|_| "La plantilla debe ser UTF-8".into())
    }
}
impl PromptTemplates for FilesystemPrompts {
    fn discover(&self) -> PromptCatalog {
        let mut catalog = PromptCatalog::default();
        let mut selected = BTreeMap::new();
        for (scope, project) in self.roots() {
            let root = prompts::project_prompt_root(&scope);
            let children = match fs::read_dir(&root) {
                Ok(children) => children,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    catalog.diagnostics.push(format!("{}: {e}", root.display()));
                    continue;
                }
            };
            let paths: Result<Vec<_>, _> = children.take(MAX_TEMPLATES + 1).collect();
            let mut paths = match paths {
                Ok(p) => p,
                Err(e) => {
                    catalog.diagnostics.push(e.to_string());
                    continue;
                }
            };
            if paths.len() > MAX_TEMPLATES {
                catalog.diagnostics.push(format!(
                    "{}: máximo {MAX_TEMPLATES} entradas",
                    root.display()
                ));
                continue;
            }
            paths.sort_by_key(|p| p.file_name());
            for child in paths {
                let path = child.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !prompts::valid_name(name) {
                    catalog
                        .diagnostics
                        .push(format!("{}: nombre inválido", path.display()));
                    continue;
                }
                match self.read(&scope, name).and_then(|text| {
                    let (description, argument_hint, _) = prompts::metadata(&text)?;
                    Ok(PromptEntry {
                        name: name.into(),
                        description,
                        argument_hint,
                        path: path.clone(),
                        project,
                    })
                }) {
                    Ok(entry) => {
                        selected.insert(name.to_string(), entry);
                    }
                    Err(e) => catalog.diagnostics.push(format!("{}: {e}", path.display())),
                }
            }
        }
        catalog.entries = selected.into_values().collect();
        catalog
    }
    fn expand(&self, input: &str) -> Result<Option<String>, String> {
        let Some((name, args)) = prompts::invocation(input)? else {
            return Ok(None);
        };
        let catalog = self.discover();
        let entry = catalog
            .entries
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| {
                format!(
                    "Plantilla no disponible: {name}. {}",
                    catalog.diagnostics.join("; ")
                )
            })?;
        let scope = if entry.project {
            &self.project
        } else {
            self.home
                .as_ref()
                .ok_or("Directorio personal no disponible")?
        };
        let text = self.read(scope, name)?;
        let (_, _, body) = prompts::metadata(&text)?;
        prompts::render(body, args).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_metadata_and_fresh_content() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = temp.path().join("repo");
        for scope in [&home, &project] {
            fs::create_dir_all(scope.join(".huginn/prompts")).unwrap();
        }
        fs::write(home.join(".huginn/prompts/review.md"), "global $1").unwrap();
        fs::write(
            project.join(".huginn/prompts/review.md"),
            "---\ndescription: Review\n---\nproject $1",
        )
        .unwrap();
        fs::write(home.join(".huginn/prompts/other.md"), "other").unwrap();
        let service = FilesystemPrompts::new(&project, Some(&home)).unwrap();
        let catalog = service.discover();
        assert_eq!(catalog.entries.len(), 2);
        assert!(
            catalog
                .entries
                .iter()
                .find(|e| e.name == "review")
                .unwrap()
                .project
        );
        assert_eq!(
            service.expand("/prompts:review café").unwrap().unwrap(),
            "project café"
        );
        fs::write(project.join(".huginn/prompts/review.md"), "fresh $1").unwrap();
        assert_eq!(
            service.expand("/prompts:review x").unwrap().unwrap(),
            "fresh x"
        );
        assert_eq!(service.expand("/login").unwrap(), None);
        assert!(service.expand("/prompts:missing").is_err());
        fs::write(project.join(".huginn/prompts/bad.md"), "---\nbad\n---\n").unwrap();
        assert!(!service.discover().diagnostics.is_empty());
    }
    #[test]
    fn oversized_directory_is_rejected_as_a_whole() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".huginn/prompts");
        fs::create_dir_all(&root).unwrap();
        for i in 0..=MAX_TEMPLATES {
            fs::write(root.join(format!("p{i}.md")), "task").unwrap();
        }
        let service = FilesystemPrompts::new(temp.path(), None).unwrap();
        let catalog = service.discover();
        assert!(catalog.entries.is_empty());
        assert_eq!(catalog.diagnostics.len(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn fifo_is_rejected_without_blocking() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".huginn/prompts");
        fs::create_dir_all(&root).unwrap();
        let status = std::process::Command::new("mkfifo")
            .arg(root.join("pipe.md"))
            .status()
            .unwrap();
        assert!(status.success());
        let service = FilesystemPrompts::new(temp.path(), None).unwrap();
        assert!(service.read(temp.path(), "pipe").is_err());
    }
    #[test]
    fn full_user_catalog_still_allows_project_override() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = temp.path().join("repo");
        for scope in [&home, &project] {
            fs::create_dir_all(scope.join(".huginn/prompts")).unwrap();
        }
        for i in 0..MAX_TEMPLATES {
            fs::write(home.join(format!(".huginn/prompts/p{i}.md")), "user").unwrap();
        }
        fs::write(project.join(".huginn/prompts/p0.md"), "project").unwrap();
        let service = FilesystemPrompts::new(&project, Some(&home)).unwrap();
        assert_eq!(service.expand("/prompts:p0").unwrap().unwrap(), "project");
    }
    #[cfg(unix)]
    #[test]
    fn scope_handle_survives_ancestor_replacement_without_reading_outside() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("repo");
        let outside = temp.path().join("outside");
        for scope in [&project, &outside] {
            fs::create_dir_all(scope.join(".huginn/prompts")).unwrap();
        }
        fs::write(project.join(".huginn/prompts/review.md"), "safe").unwrap();
        fs::write(outside.join(".huginn/prompts/review.md"), "secret").unwrap();
        let service = FilesystemPrompts::new(&project, None).unwrap();
        fs::rename(&project, temp.path().join("moved")).unwrap();
        std::os::unix::fs::symlink(&outside, &project).unwrap();
        // Exercise the descriptor reader directly: a cached scope must never
        // follow a replacement of its ambient ancestor.
        let result = service.read(&project, "review");
        assert!(result.as_ref().is_err() || result.unwrap() == "safe");
    }
    #[cfg(unix)]
    #[test]
    fn reject_symlinks_and_large_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".huginn/prompts");
        fs::create_dir_all(&root).unwrap();
        let outside = temp.path().join("secret");
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("leak.md")).unwrap();
        fs::write(root.join("large.md"), vec![b'x'; MAX_TEMPLATE_BYTES + 1]).unwrap();
        let service = FilesystemPrompts::new(temp.path(), None).unwrap();
        assert!(service.discover().entries.is_empty());
        assert!(service.expand("/prompts:leak").is_err());
        fs::remove_file(root.join("leak.md")).unwrap();
        fs::write(root.join("leak.md"), "safe").unwrap();
        assert_eq!(service.discover().entries.len(), 1);
        fs::remove_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(temp.path(), &root).unwrap();
        assert!(service.expand("/prompts:leak").is_err());
    }
}
