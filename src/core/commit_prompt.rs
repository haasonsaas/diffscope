
pub struct CommitPromptBuilder;

impl CommitPromptBuilder {
    pub fn build_commit_prompt(diff: &str) -> (String, String) {
        let system_prompt = r#"You are an expert git commit message writer. Your role is to analyze code changes and create clear, informative commit messages following the Conventional Commits specification.

Commit types:
- feat: A new feature
- fix: A bug fix
- docs: Documentation only changes
- style: Changes that don't affect code meaning (white-space, formatting)
- refactor: Code change that neither fixes a bug nor adds a feature
- perf: Performance improvement
- test: Adding or correcting tests
- build: Changes to build system or dependencies
- ci: Changes to CI configuration files and scripts
- chore: Other changes that don't modify src or test files

Format: <type>(<optional scope>): <description>

Requirements:
- First line must be under 72 characters
- Use present tense ("add" not "added")
- Don't end with a period
- Be specific about WHAT changed and WHY"#;

        let user_prompt = format!(r#"<task>
Analyze the following git diff and suggest a commit message. First, analyze the changes to understand what was modified, then generate an appropriate commit message.
</task>

<examples>
<example>
Diff: Added new user authentication module with JWT tokens
Commit: feat(auth): add JWT-based authentication system
</example>

<example>
Diff: Fixed null pointer exception in payment processing
Commit: fix(payments): handle null customer data in checkout flow
</example>

<example>
Diff: Updated README with new installation instructions
Commit: docs: update installation instructions for v2.0
</example>
</examples>

<diff>
{}
</diff>

<instructions>
1. First, analyze the diff in <analysis> tags:
   - What files were changed?
   - What is the nature of the changes (new feature, bug fix, etc.)?
   - What is the primary purpose of these changes?

2. Then provide your commit message in <commit> tags.
</instructions>"#, diff);

        (system_prompt.to_string(), user_prompt)
    }

    pub fn extract_commit_message(response: &str) -> String {
        // Try to extract from <commit> tags first
        if let Some(start) = response.find("<commit>") {
            if let Some(end) = response.find("</commit>") {
                let commit = response[start + 8..end].trim();
                return commit.to_string();
            }
        }
        
        // Fallback: take the last non-empty line that looks like a commit message
        response
            .lines()
            .rev()
            .find(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty() && 
                !trimmed.starts_with('<') &&
                !trimmed.contains("commit message") &&
                trimmed.len() < 100
            })
            .unwrap_or("")
            .trim()
            .to_string()
    }

    pub fn build_pr_title_prompt(diff: &str) -> (String, String) {
        let system_prompt = r#"You are an expert at writing clear, descriptive pull request titles. Your role is to analyze code changes and create concise PR titles that communicate the primary purpose of the changes."#;

        let user_prompt = format!(r#"<task>
Analyze the following git diff and suggest a pull request title.
</task>

<requirements>
- Maximum 65 characters
- Start with a capital letter
- Use present tense
- Be specific but concise
- Focus on the user-facing impact or main technical change
</requirements>

<examples>
<example>
Diff: Added user authentication with OAuth2
Title: Add OAuth2 authentication for user login
</example>

<example>
Diff: Fixed memory leak in image processing pipeline
Title: Fix memory leak in image processor
</example>

<example>
Diff: Refactored database queries for better performance
Title: Optimize database queries for 3x faster loading
</example>
</examples>

<diff>
{}
</diff>

<instructions>
Analyze the changes and provide a PR title in <title> tags.
</instructions>"#, diff);

        (system_prompt.to_string(), user_prompt)
    }
}