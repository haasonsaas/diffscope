use anyhow::Result;
use crate::core::{UnifiedDiff, GitIntegration};
use crate::adapters::llm::{LLMAdapter, LLMRequest};

pub struct PRSummaryGenerator;

impl PRSummaryGenerator {
    pub async fn generate_summary(
        diffs: &[UnifiedDiff],
        git: &GitIntegration,
        adapter: &Box<dyn LLMAdapter>,
    ) -> Result<PRSummary> {
        // Get commit messages for context
        let commits = git.get_recent_commits(10)?;
        
        // Analyze changes
        let stats = Self::calculate_stats(diffs);
        
        // Build prompt for AI summary
        let prompt = Self::build_summary_prompt(diffs, &commits, &stats);
        
        let request = LLMRequest {
            system_prompt: Self::get_system_prompt(),
            user_prompt: prompt,
            temperature: Some(0.3),
            max_tokens: Some(1000),
        };
        
        let response = adapter.complete(request).await?;
        
        // Parse AI response into structured summary
        Self::parse_summary_response(&response.content, stats)
    }
    
    fn calculate_stats(diffs: &[UnifiedDiff]) -> ChangeStats {
        let mut stats = ChangeStats::default();
        
        for diff in diffs {
            stats.files_changed += 1;
            
            // Categorize file type
            let extension = diff.file_path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
                
            match extension {
                "rs" | "py" | "js" | "ts" | "go" | "java" => stats.code_files += 1,
                "md" | "txt" | "rst" => stats.doc_files += 1,
                "yml" | "yaml" | "toml" | "json" => stats.config_files += 1,
                "test" | "spec" => stats.test_files += 1,
                _ => {}
            }
            
            // Count changes
            for hunk in &diff.hunks {
                for change in &hunk.changes {
                    match change.change_type {
                        crate::core::diff_parser::ChangeType::Added => stats.lines_added += 1,
                        crate::core::diff_parser::ChangeType::Removed => stats.lines_removed += 1,
                        _ => {}
                    }
                }
            }
        }
        
        stats
    }
    
    fn build_summary_prompt(
        diffs: &[UnifiedDiff],
        commits: &[String],
        stats: &ChangeStats,
    ) -> String {
        let mut prompt = String::new();
        
        prompt.push_str("Generate a comprehensive PR summary based on the following changes:\n\n");
        
        // Add statistics
        prompt.push_str(&format!("## Statistics\n"));
        prompt.push_str(&format!("- Files changed: {}\n", stats.files_changed));
        prompt.push_str(&format!("- Lines added: {}\n", stats.lines_added));
        prompt.push_str(&format!("- Lines removed: {}\n", stats.lines_removed));
        prompt.push_str(&format!("- Code files: {}\n", stats.code_files));
        prompt.push_str(&format!("- Test files: {}\n", stats.test_files));
        prompt.push_str(&format!("- Documentation: {}\n\n", stats.doc_files));
        
        // Add recent commits
        if !commits.is_empty() {
            prompt.push_str("## Recent Commits\n");
            for commit in commits.iter().take(5) {
                prompt.push_str(&format!("- {}\n", commit));
            }
            prompt.push_str("\n");
        }
        
        // Add file changes summary
        prompt.push_str("## Files Changed\n");
        for diff in diffs {
            let path = diff.file_path.display();
            let added = diff.hunks.iter()
                .flat_map(|h| &h.changes)
                .filter(|c| matches!(c.change_type, crate::core::diff_parser::ChangeType::Added))
                .count();
            let removed = diff.hunks.iter()
                .flat_map(|h| &h.changes)
                .filter(|c| matches!(c.change_type, crate::core::diff_parser::ChangeType::Removed))
                .count();
                
            prompt.push_str(&format!("- {} (+{}, -{})\n", path, added, removed));
        }
        
        prompt.push_str("\n## Instructions\n");
        prompt.push_str("Create a structured summary with:\n");
        prompt.push_str("1. A brief one-line description\n");
        prompt.push_str("2. Key changes (3-5 bullet points)\n");
        prompt.push_str("3. Type of change (feature/fix/refactor/docs)\n");
        prompt.push_str("4. Breaking changes (if any)\n");
        prompt.push_str("5. Testing notes\n");
        
        prompt
    }
    
    fn get_system_prompt() -> String {
        r#"You are an AI assistant that generates clear, concise PR summaries.
        
Focus on:
- What changed and why
- Impact on users/developers
- Key technical details
- Testing considerations

Format your response as:
SUMMARY: [one line description]
TYPE: [feature|fix|refactor|docs|test|chore]
KEY_CHANGES:
- [change 1]
- [change 2]
- [change 3]
BREAKING_CHANGES: [none or describe]
TESTING_NOTES: [what to test]"#.to_string()
    }
    
    fn parse_summary_response(content: &str, stats: ChangeStats) -> Result<PRSummary> {
        let mut summary = PRSummary {
            title: String::new(),
            description: String::new(),
            change_type: ChangeType::Feature,
            key_changes: Vec::new(),
            breaking_changes: None,
            testing_notes: String::new(),
            stats,
            _visual_diff: None,
        };
        
        // Parse structured response
        let lines: Vec<&str> = content.lines().collect();
        let mut current_section = "";
        
        for line in lines {
            let line = line.trim();
            
            if line.starts_with("SUMMARY:") {
                summary.title = line.strip_prefix("SUMMARY:").unwrap_or("").trim().to_string();
            } else if line.starts_with("TYPE:") {
                let type_str = line.strip_prefix("TYPE:").unwrap_or("").trim();
                summary.change_type = match type_str {
                    "fix" => ChangeType::Fix,
                    "refactor" => ChangeType::Refactor,
                    "docs" => ChangeType::Docs,
                    "test" => ChangeType::Test,
                    "chore" => ChangeType::Chore,
                    _ => ChangeType::Feature,
                };
            } else if line.starts_with("KEY_CHANGES:") {
                current_section = "changes";
            } else if line.starts_with("BREAKING_CHANGES:") {
                let breaking = line.strip_prefix("BREAKING_CHANGES:").unwrap_or("").trim();
                if breaking != "none" && !breaking.is_empty() {
                    summary.breaking_changes = Some(breaking.to_string());
                }
            } else if line.starts_with("TESTING_NOTES:") {
                summary.testing_notes = line.strip_prefix("TESTING_NOTES:").unwrap_or("").trim().to_string();
            } else if current_section == "changes" && line.starts_with("- ") {
                summary.key_changes.push(line.strip_prefix("- ").unwrap_or("").to_string());
            }
        }
        
        // Build description from key changes
        if !summary.key_changes.is_empty() {
            summary.description = summary.key_changes.join("\n");
        }
        
        Ok(summary)
    }
}

#[derive(Debug, Clone)]
pub struct PRSummary {
    pub title: String,
    pub description: String,
    pub change_type: ChangeType,
    pub key_changes: Vec<String>,
    pub breaking_changes: Option<String>,
    pub testing_notes: String,
    pub stats: ChangeStats,
    pub _visual_diff: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ChangeType {
    Feature,
    Fix,
    Refactor,
    Docs,
    Test,
    Chore,
}

#[derive(Debug, Clone, Default)]
pub struct ChangeStats {
    pub files_changed: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub code_files: usize,
    pub test_files: usize,
    pub doc_files: usize,
    pub config_files: usize,
}

impl PRSummary {
    pub fn to_markdown(&self) -> String {
        let mut output = String::new();
        
        // Title and type badge
        let type_emoji = match self.change_type {
            ChangeType::Feature => "✨",
            ChangeType::Fix => "🐛",
            ChangeType::Refactor => "♻️",
            ChangeType::Docs => "📚",
            ChangeType::Test => "🧪",
            ChangeType::Chore => "🔧",
        };
        
        output.push_str(&format!("# {} {}\n\n", type_emoji, self.title));
        
        // Description
        if !self.description.is_empty() {
            output.push_str(&format!("{}\n\n", self.description));
        }
        
        // Key changes
        if !self.key_changes.is_empty() {
            output.push_str("## 🎯 Key Changes\n\n");
            for change in &self.key_changes {
                output.push_str(&format!("- {}\n", change));
            }
            output.push_str("\n");
        }
        
        // Statistics
        output.push_str("## 📊 Change Statistics\n\n");
        output.push_str(&format!("- **Files Changed:** {}\n", self.stats.files_changed));
        output.push_str(&format!("- **Lines Added:** {} +++\n", self.stats.lines_added));
        output.push_str(&format!("- **Lines Removed:** {} ---\n", self.stats.lines_removed));
        
        if self.stats.test_files > 0 {
            output.push_str(&format!("- **Tests Modified:** {} files\n", self.stats.test_files));
        }
        if self.stats.doc_files > 0 {
            output.push_str(&format!("- **Docs Updated:** {} files\n", self.stats.doc_files));
        }
        output.push_str("\n");
        
        // Breaking changes
        if let Some(breaking) = &self.breaking_changes {
            output.push_str("## ⚠️ Breaking Changes\n\n");
            output.push_str(&format!("{}\n\n", breaking));
        }
        
        // Testing notes
        if !self.testing_notes.is_empty() {
            output.push_str("## 🧪 Testing Notes\n\n");
            output.push_str(&format!("{}\n\n", self.testing_notes));
        }
        
        output
    }
}