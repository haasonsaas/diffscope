use crate::core::{LLMContextChunk, UnifiedDiff};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptConfig {
    pub system_prompt: String,
    pub user_prompt_template: String,
    pub max_tokens: usize,
    pub include_context: bool,
    pub max_context_chars: usize,
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self {
            system_prompt: r#"You are an expert code reviewer with deep knowledge of software security, performance optimization, and best practices. Your role is to identify critical issues in code changes that could impact:
- Security (vulnerabilities, data exposure, injection risks)
- Correctness (bugs, logic errors, edge cases)
- Performance (inefficiencies, memory leaks, algorithmic complexity)
- Maintainability (code clarity, error handling, documentation)

Focus only on actionable issues. Do not comment on code style or formatting unless it impacts functionality."#.to_string(),
            user_prompt_template: r#"<task>
Review the code changes below and identify specific issues. Focus on problems that could cause bugs, security vulnerabilities, or performance issues.
</task>

<diff>
{diff}
</diff>

<context>
{context}
</context>

<instructions>
1. Analyze the changes systematically
2. For each issue found, provide:
   - Line number where the issue occurs
   - Clear description of the problem
   - Impact if not addressed
   - Suggested fix (if applicable)

Format each issue as:
Line [number]: [Issue type] - [Description]. [Impact]. [Suggestion if applicable].

Examples:
Line 42: Security - User input passed directly to SQL query. Risk of SQL injection. Use parameterized queries.
Line 13: Bug - Missing null check before dereferencing pointer. May cause crash. Add null validation.
Line 28: Performance - O(n²) algorithm for large dataset. Will be slow with many items. Consider using a hash map.
</instructions>"#.to_string(),
            max_tokens: 2000,
            include_context: true,
            max_context_chars: 20000,
        }
    }
}

pub struct PromptBuilder {
    config: PromptConfig,
}

impl PromptBuilder {
    pub fn new(config: PromptConfig) -> Self {
        Self { config }
    }

    pub fn build_prompt(
        &self,
        diff: &UnifiedDiff,
        context_chunks: &[LLMContextChunk],
    ) -> Result<(String, String)> {
        let diff_text = self.format_diff(diff)?;
        let context_text = if self.config.include_context {
            self.format_context(context_chunks)?
        } else {
            String::new()
        };

        let user_prompt = self
            .config
            .user_prompt_template
            .replace("{diff}", &diff_text)
            .replace("{context}", &context_text);

        Ok((self.config.system_prompt.clone(), user_prompt))
    }

    fn format_diff(&self, diff: &UnifiedDiff) -> Result<String> {
        let mut output = String::new();
        output.push_str(&format!("File: {}\n", diff.file_path.display()));

        for hunk in &diff.hunks {
            output.push_str(&format!("{}\n", hunk.context));

            for change in &hunk.changes {
                let prefix = match change.change_type {
                    crate::core::diff_parser::ChangeType::Added => "+",
                    crate::core::diff_parser::ChangeType::Removed => "-",
                    crate::core::diff_parser::ChangeType::Context => " ",
                };
                output.push_str(&format!("{}{}\n", prefix, change.content));
            }
        }

        Ok(output)
    }

    fn format_context(&self, chunks: &[LLMContextChunk]) -> Result<String> {
        let mut output = String::new();

        for chunk in chunks {
            let block = format!(
                "\n[{:?} - {}{}]\n{}\n",
                chunk.context_type,
                chunk.file_path.display(),
                chunk
                    .line_range
                    .map(|(s, e)| format!(":{}-{}", s, e))
                    .unwrap_or_default(),
                chunk.content
            );
            if self.config.max_context_chars > 0
                && output.len().saturating_add(block.len()) > self.config.max_context_chars
            {
                output.push_str("\n[Context truncated]\n");
                break;
            }
            output.push_str(&block);
        }

        Ok(output)
    }
}
