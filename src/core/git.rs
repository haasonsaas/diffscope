use anyhow::{Context, Result};
use git2::{BranchType, DiffFormat, DiffOptions, Repository};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};

/// Validate a user-provided git ref name (branch, tag, or arbitrary ref).
///
/// Git ref names must not contain spaces, `..`, control characters, or
/// the special characters `~`, `^`, `:`, `?`, `\`, `[`, `*`.
/// They also cannot start/end with `.` or `/`, and cannot end with `.lock`.
pub fn validate_ref_name(name: &str) -> Result<()> {
    static REF_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_./@{}\-]*$").unwrap());

    if name.is_empty() {
        anyhow::bail!("Invalid ref name: name is empty");
    }

    if !REF_RE.is_match(name) {
        anyhow::bail!(
            "Invalid ref name '{}': must start with alphanumeric and contain only \
             alphanumeric, '.', '_', '/', '@', '{{', '}}', or '-' characters",
            name
        );
    }

    if name.contains("..") {
        anyhow::bail!("Invalid ref name '{}': must not contain '..'", name);
    }

    if name.ends_with(".lock") {
        anyhow::bail!("Invalid ref name '{}': must not end with '.lock'", name);
    }

    if name.ends_with('/') || name.ends_with('.') {
        anyhow::bail!("Invalid ref name '{}': must not end with '/' or '.'", name);
    }

    Ok(())
}

pub struct GitIntegration {
    repo: Repository,
}

impl GitIntegration {
    pub fn new(repo_path: impl AsRef<Path>) -> Result<Self> {
        let repo = Repository::discover(repo_path).context("Failed to find git repository")?;
        Ok(Self { repo })
    }

    pub fn get_uncommitted_diff(&self) -> Result<String> {
        let mut diff_options = DiffOptions::new();
        diff_options.include_untracked(true);

        let head = self.repo.head()?.peel_to_tree()?;
        let diff = self
            .repo
            .diff_tree_to_workdir_with_index(Some(&head), Some(&mut diff_options))?;

        let mut diff_text = Vec::new();
        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            diff_text.extend_from_slice(line.content());
            true
        })?;

        Ok(String::from_utf8_lossy(&diff_text).to_string())
    }

    pub fn get_staged_diff(&self) -> Result<String> {
        let head = self.repo.head()?.peel_to_tree()?;
        let mut index = self.repo.index()?;
        let oid = index.write_tree()?;
        let index_tree = self.repo.find_tree(oid)?;

        let diff = self
            .repo
            .diff_tree_to_tree(Some(&head), Some(&index_tree), None)?;

        let mut diff_text = Vec::new();
        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            diff_text.extend_from_slice(line.content());
            true
        })?;

        Ok(String::from_utf8_lossy(&diff_text).to_string())
    }

    pub fn get_branch_diff(&self, base_branch: &str) -> Result<String> {
        validate_ref_name(base_branch)?;
        let base = self.repo.revparse_single(base_branch)?.peel_to_commit()?;
        let head = self.repo.head()?.peel_to_commit()?;

        let base_tree = base.tree()?;
        let head_tree = head.tree()?;

        let diff = self
            .repo
            .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None)?;

        let mut diff_text = Vec::new();
        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            diff_text.extend_from_slice(line.content());
            true
        })?;

        Ok(String::from_utf8_lossy(&diff_text).to_string())
    }

    pub fn get_current_branch(&self) -> Result<String> {
        let head = self.repo.head()?;
        Ok(head.shorthand().unwrap_or("HEAD").to_string())
    }

    pub fn get_remote_url(&self) -> Result<Option<String>> {
        let remote = self.repo.find_remote("origin")?;
        Ok(remote.url().ok().map(|url| url.to_string()))
    }

    pub fn get_recent_commits(&self, count: usize) -> Result<Vec<String>> {
        let mut revwalk = self.repo.revwalk()?;
        revwalk.push_head()?;

        let mut commits = Vec::new();
        for (i, oid) in revwalk.enumerate() {
            if i >= count {
                break;
            }

            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            let summary = commit
                .summary()
                .ok()
                .flatten()
                .unwrap_or("No commit message");
            commits.push(summary.to_string());
        }

        Ok(commits)
    }

    pub fn workdir(&self) -> Option<PathBuf> {
        self.repo.workdir().map(|path| path.to_path_buf())
    }

    pub fn get_default_branch(&self) -> Result<String> {
        if let Ok(reference) = self.repo.find_reference("refs/remotes/origin/HEAD") {
            if let Some(target) = reference.symbolic_target().ok().flatten() {
                if let Some(branch) = target.rsplit('/').next() {
                    return Ok(branch.to_string());
                }
            }
        }

        if self.repo.find_branch("main", BranchType::Local).is_ok() {
            return Ok("main".to_string());
        }
        if self.repo.find_branch("master", BranchType::Local).is_ok() {
            return Ok("master".to_string());
        }

        Ok("main".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_ref_name_accepts_simple_branch() {
        assert!(validate_ref_name("main").is_ok());
        assert!(validate_ref_name("master").is_ok());
        assert!(validate_ref_name("develop").is_ok());
    }

    #[test]
    fn validate_ref_name_accepts_slashes_and_dashes() {
        assert!(validate_ref_name("feature/add-login").is_ok());
        assert!(validate_ref_name("release/v1.2.3").is_ok());
        assert!(validate_ref_name("fix/issue-42").is_ok());
    }

    #[test]
    fn validate_ref_name_accepts_at_brace_syntax() {
        // git reflog syntax like HEAD@{1}
        assert!(validate_ref_name("HEAD@{1}").is_ok());
    }

    #[test]
    fn validate_ref_name_rejects_empty() {
        assert!(validate_ref_name("").is_err());
    }

    #[test]
    fn validate_ref_name_rejects_double_dot() {
        assert!(validate_ref_name("main..develop").is_err());
    }

    #[test]
    fn validate_ref_name_rejects_dot_lock_suffix() {
        assert!(validate_ref_name("branch.lock").is_err());
    }

    #[test]
    fn validate_ref_name_rejects_trailing_slash() {
        assert!(validate_ref_name("feature/").is_err());
    }

    #[test]
    fn validate_ref_name_rejects_trailing_dot() {
        assert!(validate_ref_name("branch.").is_err());
    }

    #[test]
    fn validate_ref_name_rejects_spaces() {
        assert!(validate_ref_name("my branch").is_err());
    }

    #[test]
    fn validate_ref_name_rejects_control_chars() {
        assert!(validate_ref_name("branch\x00name").is_err());
        assert!(validate_ref_name("branch\tname").is_err());
    }

    #[test]
    fn validate_ref_name_rejects_special_git_chars() {
        assert!(validate_ref_name("branch~1").is_err());
        assert!(validate_ref_name("branch^2").is_err());
        assert!(validate_ref_name("branch:ref").is_err());
        assert!(validate_ref_name("branch?").is_err());
        assert!(validate_ref_name("branch[0]").is_err());
        assert!(validate_ref_name("branch*").is_err());
    }

    #[test]
    fn validate_ref_name_rejects_starting_with_dot() {
        assert!(validate_ref_name(".hidden").is_err());
    }

    #[test]
    fn validate_ref_name_rejects_starting_with_dash() {
        assert!(validate_ref_name("-flag").is_err());
    }
}
