use anyhow::{Context, Result};
use git2::{BranchType, DiffFormat, DiffOptions, Repository};
use std::path::{Path, PathBuf};

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
        if let Some(name) = head.shorthand() {
            Ok(name.to_string())
        } else {
            Ok("HEAD".to_string())
        }
    }

    pub fn get_remote_url(&self) -> Result<Option<String>> {
        let remote = self.repo.find_remote("origin")?;
        Ok(remote.url().map(|s| s.to_string()))
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
            let summary = commit.summary().unwrap_or("No commit message");
            commits.push(summary.to_string());
        }

        Ok(commits)
    }

    pub fn workdir(&self) -> Option<PathBuf> {
        self.repo.workdir().map(|path| path.to_path_buf())
    }

    pub fn get_default_branch(&self) -> Result<String> {
        if let Ok(reference) = self.repo.find_reference("refs/remotes/origin/HEAD") {
            if let Some(target) = reference.symbolic_target() {
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
