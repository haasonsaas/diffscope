use anyhow::Result;
use chrono::{DateTime, Local};
use git2::Repository;
use regex::Regex;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ChangelogEntry {
    pub commit_hash: String,
    pub message: String,
    pub author: String,
    pub _date: DateTime<Local>,
    pub change_type: ChangeType,
    pub scope: Option<String>,
    pub breaking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChangeType {
    Feature,
    Fix,
    Docs,
    Style,
    Refactor,
    Perf,
    Test,
    Build,
    Ci,
    Chore,
    Revert,
}

impl ChangeType {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "feat" | "feature" => Self::Feature,
            "fix" | "bugfix" => Self::Fix,
            "docs" | "documentation" => Self::Docs,
            "style" => Self::Style,
            "refactor" => Self::Refactor,
            "perf" | "performance" => Self::Perf,
            "test" | "tests" => Self::Test,
            "build" => Self::Build,
            "ci" => Self::Ci,
            "chore" => Self::Chore,
            "revert" => Self::Revert,
            _ => Self::Chore,
        }
    }

    fn emoji(&self) -> &'static str {
        match self {
            Self::Feature => "✨",
            Self::Fix => "🐛",
            Self::Docs => "📚",
            Self::Style => "💄",
            Self::Refactor => "♻️",
            Self::Perf => "⚡",
            Self::Test => "✅",
            Self::Build => "📦",
            Self::Ci => "👷",
            Self::Chore => "🔧",
            Self::Revert => "⏪",
        }
    }

    fn heading(&self) -> &'static str {
        match self {
            Self::Feature => "Features",
            Self::Fix => "Bug Fixes",
            Self::Docs => "Documentation",
            Self::Style => "Styles",
            Self::Refactor => "Code Refactoring",
            Self::Perf => "Performance Improvements",
            Self::Test => "Tests",
            Self::Build => "Build System",
            Self::Ci => "Continuous Integration",
            Self::Chore => "Chores",
            Self::Revert => "Reverts",
        }
    }
}

pub struct ChangelogGenerator {
    repo: Repository,
    conventional_regex: Regex,
}

impl ChangelogGenerator {
    pub fn new(repo_path: &str) -> Result<Self> {
        let repo = Repository::discover(repo_path)?;
        let conventional_regex = Regex::new(
            r"^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(?:\(([^)]+)\))?(?:!)?:\s*(.+)",
        )?;

        Ok(Self {
            repo,
            conventional_regex,
        })
    }

    pub fn generate_changelog(&self, from_tag: Option<&str>, to_ref: &str) -> Result<String> {
        let entries = self.collect_entries(from_tag, to_ref)?;
        Ok(self.format_changelog(&entries, from_tag, to_ref))
    }

    pub fn generate_release_notes(&self, version: &str, from_tag: Option<&str>) -> Result<String> {
        let entries = self.collect_entries(from_tag, "HEAD")?;
        Ok(self.format_release_notes(&entries, version))
    }

    fn collect_entries(&self, from_tag: Option<&str>, to_ref: &str) -> Result<Vec<ChangelogEntry>> {
        let mut revwalk = self.repo.revwalk()?;

        // Start from the target ref
        let to_oid = self.repo.revparse_single(to_ref)?.id();
        revwalk.push(to_oid)?;

        // Exclude commits from the starting point if provided
        let _from_oid = if let Some(tag) = from_tag {
            let oid = self.repo.revparse_single(tag)?.id();
            revwalk.hide(oid)?;
            Some(oid)
        } else {
            None
        };

        let mut entries = Vec::new();

        for oid in revwalk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;

            // Skip merge commits
            if commit.parent_count() > 1 {
                continue;
            }

            if let Some(entry) = self.parse_commit(&commit)? {
                entries.push(entry);
            }
        }

        entries.reverse(); // Show oldest first
        Ok(entries)
    }

    fn parse_commit(&self, commit: &git2::Commit) -> Result<Option<ChangelogEntry>> {
        let message = commit.message().unwrap_or("");
        let first_line = message.lines().next().unwrap_or("");

        // Try to parse as conventional commit
        if let Some(captures) = self.conventional_regex.captures(first_line) {
            let change_type = ChangeType::from_str(captures.get(1).unwrap().as_str());
            let scope = captures.get(2).map(|m| m.as_str().to_string());
            let description = captures.get(3).unwrap().as_str().to_string();
            let breaking = first_line.contains('!') || message.contains("BREAKING CHANGE");

            Ok(Some(ChangelogEntry {
                commit_hash: format!("{:.7}", commit.id()),
                message: description,
                author: commit.author().name().unwrap_or("Unknown").to_string(),
                _date: DateTime::from_timestamp(commit.time().seconds(), 0)
                    .unwrap_or_default()
                    .with_timezone(&Local),
                change_type,
                scope,
                breaking,
            }))
        } else {
            // Non-conventional commit - try to categorize
            let change_type = if first_line.to_lowercase().contains("fix") {
                ChangeType::Fix
            } else if first_line.to_lowercase().contains("add") {
                ChangeType::Feature
            } else {
                ChangeType::Chore
            };

            Ok(Some(ChangelogEntry {
                commit_hash: format!("{:.7}", commit.id()),
                message: first_line.to_string(),
                author: commit.author().name().unwrap_or("Unknown").to_string(),
                _date: DateTime::from_timestamp(commit.time().seconds(), 0)
                    .unwrap_or_default()
                    .with_timezone(&Local),
                change_type,
                scope: None,
                breaking: false,
            }))
        }
    }

    fn format_changelog(
        &self,
        entries: &[ChangelogEntry],
        from_tag: Option<&str>,
        to_ref: &str,
    ) -> String {
        let mut output = String::new();

        // Header
        output.push_str("# Changelog\n\n");

        let _date = Local::now().format("%Y-%m-%d");
        output.push_str(&format!(
            "## [{} - {}]\n\n",
            from_tag.unwrap_or("Start"),
            to_ref
        ));

        // Group by type
        let mut grouped: HashMap<ChangeType, Vec<&ChangelogEntry>> = HashMap::new();
        let mut breaking_changes = Vec::new();

        for entry in entries {
            if entry.breaking {
                breaking_changes.push(entry);
            }
            grouped
                .entry(entry.change_type.clone())
                .or_default()
                .push(entry);
        }

        // Breaking changes first
        if !breaking_changes.is_empty() {
            output.push_str("### ⚠️ BREAKING CHANGES\n\n");
            for entry in &breaking_changes {
                output.push_str(&format!("* {}\n", entry.message));
            }
            output.push('\n');
        }

        // Then by category
        let type_order = [
            ChangeType::Feature,
            ChangeType::Fix,
            ChangeType::Perf,
            ChangeType::Refactor,
            ChangeType::Docs,
            ChangeType::Test,
            ChangeType::Build,
            ChangeType::Ci,
            ChangeType::Style,
            ChangeType::Chore,
        ];

        for change_type in &type_order {
            if let Some(entries) = grouped.get(change_type) {
                if !entries.is_empty() {
                    output.push_str(&format!(
                        "### {} {}\n\n",
                        change_type.emoji(),
                        change_type.heading()
                    ));

                    for entry in entries {
                        if let Some(scope) = &entry.scope {
                            output.push_str(&format!(
                                "* **{}**: {} ({})\n",
                                scope, entry.message, entry.commit_hash
                            ));
                        } else {
                            output.push_str(&format!(
                                "* {} ({})\n",
                                entry.message, entry.commit_hash
                            ));
                        }
                    }
                    output.push('\n');
                }
            }
        }

        output
    }

    fn format_release_notes(&self, entries: &[ChangelogEntry], version: &str) -> String {
        let mut output = String::new();

        // Header
        output.push_str(&format!("# Release Notes - v{}\n\n", version));
        output.push_str(&format!(
            "📅 **Release Date**: {}\n\n",
            Local::now().format("%Y-%m-%d")
        ));

        // Summary statistics
        let features = entries
            .iter()
            .filter(|e| matches!(e.change_type, ChangeType::Feature))
            .count();
        let fixes = entries
            .iter()
            .filter(|e| matches!(e.change_type, ChangeType::Fix))
            .count();
        let breaking = entries.iter().filter(|e| e.breaking).count();

        output.push_str("## 📊 Summary\n\n");
        output.push_str(&format!("- 🎯 **Total Changes**: {}\n", entries.len()));
        output.push_str(&format!("- ✨ **New Features**: {}\n", features));
        output.push_str(&format!("- 🐛 **Bug Fixes**: {}\n", fixes));
        if breaking > 0 {
            output.push_str(&format!("- ⚠️  **Breaking Changes**: {}\n", breaking));
        }
        output.push('\n');

        // Highlights (features and breaking changes)
        let feature_entries: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e.change_type, ChangeType::Feature))
            .collect();

        if !feature_entries.is_empty() {
            output.push_str("## ✨ Highlights\n\n");
            for entry in feature_entries.iter().take(5) {
                output.push_str(&format!("- {}\n", entry.message));
            }
            output.push('\n');
        }

        // Breaking changes
        let breaking_entries: Vec<_> = entries.iter().filter(|e| e.breaking).collect();

        if !breaking_entries.is_empty() {
            output.push_str("## ⚠️ Breaking Changes\n\n");
            for entry in &breaking_entries {
                output.push_str(&format!("- {}\n", entry.message));
            }
            output.push('\n');
        }

        // Bug fixes
        let fix_entries: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e.change_type, ChangeType::Fix))
            .collect();

        if !fix_entries.is_empty() {
            output.push_str("## 🐛 Bug Fixes\n\n");
            for entry in fix_entries.iter().take(10) {
                output.push_str(&format!("- {}\n", entry.message));
            }
            output.push('\n');
        }

        // Contributors
        let mut contributors: HashMap<String, usize> = HashMap::new();
        for entry in entries {
            *contributors.entry(entry.author.clone()).or_default() += 1;
        }

        let mut contributors: Vec<_> = contributors.into_iter().collect();
        contributors.sort_by(|a, b| b.1.cmp(&a.1));

        output.push_str("## 👥 Contributors\n\n");
        output.push_str("Thank you to all contributors:\n\n");
        for (author, count) in contributors.iter().take(10) {
            output.push_str(&format!("- {} ({} commits)\n", author, count));
        }

        output
    }
}
