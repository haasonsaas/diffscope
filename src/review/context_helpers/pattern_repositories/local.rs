use std::path::{Path, PathBuf};

pub(super) fn resolve_local_repository_path(source: &str, repo_root: &Path) -> Option<PathBuf> {
    let repo_root = repo_root.canonicalize().ok()?;
    let source_path = Path::new(source);
    if source_path.is_absolute() {
        let source_path = source_path.canonicalize().ok()?;
        if source_path.is_dir() && source_path.starts_with(&repo_root) {
            return Some(source_path);
        }
        return None;
    }

    let repo_relative = repo_root.join(source);
    if repo_relative.is_dir() {
        let repo_relative = repo_relative.canonicalize().ok()?;
        if repo_relative.starts_with(&repo_root) {
            return Some(repo_relative);
        }
    }

    None
}
