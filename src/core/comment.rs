use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    #[serde(default)]
    pub id: String,
    pub file_path: PathBuf,
    pub line_number: usize,
    pub content: String,
    pub severity: Severity,
    pub category: Category,
    pub suggestion: Option<String>,
    pub confidence: f32,
    pub code_suggestion: Option<CodeSuggestion>,
    pub tags: Vec<String>,
    pub fix_effort: FixEffort,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSuggestion {
    pub original_code: String,
    pub suggested_code: String,
    pub explanation: String,
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSummary {
    pub total_comments: usize,
    pub by_severity: HashMap<String, usize>,
    pub by_category: HashMap<String, usize>,
    pub critical_issues: usize,
    pub files_reviewed: usize,
    pub overall_score: f32,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Suggestion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Category {
    Bug,
    Security,
    Performance,
    Style,
    Documentation,
    BestPractice,
    Maintainability,
    Testing,
    Architecture,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FixEffort {
    Low,    // < 5 minutes
    Medium, // 5-30 minutes
    High,   // > 30 minutes
}

pub struct CommentSynthesizer;

impl CommentSynthesizer {
    pub fn synthesize(raw_comments: Vec<RawComment>) -> Result<Vec<Comment>> {
        let mut comments = Vec::new();

        for raw in raw_comments {
            if let Some(comment) = Self::process_raw_comment(raw)? {
                comments.push(comment);
            }
        }

        Self::deduplicate_comments(&mut comments);
        Self::sort_by_priority(&mut comments);

        Ok(comments)
    }

    pub fn generate_summary(comments: &[Comment]) -> ReviewSummary {
        let mut by_severity = HashMap::new();
        let mut by_category = HashMap::new();
        let mut files = std::collections::HashSet::new();
        let mut critical_issues = 0;

        for comment in comments {
            let severity_str = format!("{:?}", comment.severity);
            *by_severity.entry(severity_str).or_insert(0) += 1;

            let category_str = format!("{:?}", comment.category);
            *by_category.entry(category_str).or_insert(0) += 1;

            files.insert(comment.file_path.clone());

            if matches!(comment.severity, Severity::Error) {
                critical_issues += 1;
            }
        }

        let overall_score = Self::calculate_overall_score(comments);
        let recommendations = Self::generate_recommendations(comments);

        ReviewSummary {
            total_comments: comments.len(),
            by_severity,
            by_category,
            critical_issues,
            files_reviewed: files.len(),
            overall_score,
            recommendations,
        }
    }

    fn process_raw_comment(raw: RawComment) -> Result<Option<Comment>> {
        let severity = raw
            .severity
            .clone()
            .unwrap_or_else(|| Self::determine_severity(&raw.content));
        let category = raw
            .category
            .clone()
            .unwrap_or_else(|| Self::determine_category(&raw.content));
        let confidence = raw
            .confidence
            .unwrap_or_else(|| Self::calculate_confidence(&raw.content, &severity, &category));
        let confidence = confidence.clamp(0.0, 1.0);
        let tags = if raw.tags.is_empty() {
            Self::extract_tags(&raw.content, &category)
        } else {
            raw.tags.clone()
        };
        let fix_effort = raw
            .fix_effort
            .clone()
            .unwrap_or_else(|| Self::determine_fix_effort(&raw.content, &category));
        let code_suggestion = Self::generate_code_suggestion(&raw);
        let id = Self::generate_comment_id(&raw.file_path, &raw.content, &category);

        Ok(Some(Comment {
            id,
            file_path: raw.file_path,
            line_number: raw.line_number,
            content: raw.content,
            severity,
            category,
            suggestion: raw.suggestion,
            confidence,
            code_suggestion,
            tags,
            fix_effort,
        }))
    }

    fn generate_comment_id(file_path: &Path, content: &str, category: &Category) -> String {
        compute_comment_id(file_path, content, category)
    }

    fn determine_severity(content: &str) -> Severity {
        let lower = content.to_lowercase();
        if lower.contains("error") || lower.contains("critical") {
            Severity::Error
        } else if lower.contains("warning") || lower.contains("issue") {
            Severity::Warning
        } else if lower.contains("consider") || lower.contains("suggestion") {
            Severity::Suggestion
        } else {
            Severity::Info
        }
    }

    fn determine_category(content: &str) -> Category {
        let lower = content.to_lowercase();
        if lower.contains("security")
            || lower.contains("vulnerability")
            || lower.contains("injection")
        {
            Category::Security
        } else if lower.contains("performance")
            || lower.contains("optimization")
            || lower.contains("slow")
        {
            Category::Performance
        } else if lower.contains("bug") || lower.contains("fix") || lower.contains("error") {
            Category::Bug
        } else if lower.contains("style") || lower.contains("format") || lower.contains("naming") {
            Category::Style
        } else if lower.contains("doc") || lower.contains("comment") {
            Category::Documentation
        } else if lower.contains("test") || lower.contains("coverage") {
            Category::Testing
        } else if lower.contains("maintain")
            || lower.contains("complex")
            || lower.contains("readable")
        {
            Category::Maintainability
        } else if lower.contains("design")
            || lower.contains("architecture")
            || lower.contains("pattern")
        {
            Category::Architecture
        } else {
            Category::BestPractice
        }
    }

    fn calculate_confidence(content: &str, severity: &Severity, _category: &Category) -> f32 {
        let mut confidence: f32 = 0.7; // Base confidence

        // Boost confidence for specific patterns
        let lower = content.to_lowercase();
        if lower.contains("sql injection") || lower.contains("xss") || lower.contains("csrf") {
            confidence += 0.2;
        }
        if lower.contains("null pointer") || lower.contains("buffer overflow") {
            confidence += 0.2;
        }
        if lower.contains("performance issue") || lower.contains("n+1") {
            confidence += 0.15;
        }

        // Adjust based on severity
        match severity {
            Severity::Error => confidence += 0.1,
            Severity::Warning => confidence += 0.05,
            _ => {}
        }

        // Ensure confidence stays in bounds
        confidence.clamp(0.1, 1.0)
    }

    fn extract_tags(content: &str, category: &Category) -> Vec<String> {
        let mut tags = vec![format!("{:?}", category).to_lowercase()];
        let lower = content.to_lowercase();

        // Security-specific tags
        if lower.contains("sql") {
            tags.push("sql".to_string());
        }
        if lower.contains("injection") {
            tags.push("injection".to_string());
        }
        if lower.contains("xss") {
            tags.push("xss".to_string());
        }
        if lower.contains("csrf") {
            tags.push("csrf".to_string());
        }
        if lower.contains("auth") {
            tags.push("authentication".to_string());
        }

        // Performance tags
        if lower.contains("n+1") {
            tags.push("n+1-query".to_string());
        }
        if lower.contains("memory") {
            tags.push("memory".to_string());
        }
        if lower.contains("cache") {
            tags.push("caching".to_string());
        }

        // Code quality tags
        if lower.contains("duplicate") {
            tags.push("duplication".to_string());
        }
        if lower.contains("complex") {
            tags.push("complexity".to_string());
        }
        if lower.contains("deprecated") {
            tags.push("deprecated".to_string());
        }

        tags
    }

    fn determine_fix_effort(content: &str, category: &Category) -> FixEffort {
        let lower = content.to_lowercase();

        // High effort indicators
        if lower.contains("architecture")
            || lower.contains("refactor")
            || lower.contains("redesign")
        {
            return FixEffort::High;
        }

        // Security issues often require careful consideration
        if matches!(category, Category::Security)
            && (lower.contains("injection") || lower.contains("vulnerability"))
        {
            return FixEffort::Medium;
        }

        // Performance issues might need investigation
        if matches!(category, Category::Performance) && lower.contains("n+1") {
            return FixEffort::Medium;
        }

        // Style and documentation are usually quick fixes
        if matches!(category, Category::Style | Category::Documentation) {
            return FixEffort::Low;
        }

        FixEffort::Medium
    }

    fn generate_code_suggestion(raw: &RawComment) -> Option<CodeSuggestion> {
        // This is a simplified implementation - in practice, you'd use the LLM
        // to generate more sophisticated code suggestions
        if let Some(suggestion) = &raw.suggestion {
            if suggestion.contains("use") || suggestion.contains("replace") {
                return Some(CodeSuggestion {
                    original_code: "// Original code would be extracted from context".to_string(),
                    suggested_code: suggestion.clone(),
                    explanation: "Improved implementation following best practices".to_string(),
                    diff: format!("- original\n+ {}", suggestion),
                });
            }
        }
        None
    }

    fn calculate_overall_score(comments: &[Comment]) -> f32 {
        if comments.is_empty() {
            return 10.0;
        }

        let mut score: f32 = 10.0;
        for comment in comments {
            let penalty = match comment.severity {
                Severity::Error => 2.0,
                Severity::Warning => 1.0,
                Severity::Info => 0.3,
                Severity::Suggestion => 0.1,
            };
            score -= penalty;
        }

        score.clamp(0.0, 10.0)
    }

    fn generate_recommendations(comments: &[Comment]) -> Vec<String> {
        let mut recommendations = Vec::new();
        let mut security_count = 0;
        let mut performance_count = 0;
        let mut style_count = 0;

        for comment in comments {
            match comment.category {
                Category::Security => security_count += 1,
                Category::Performance => performance_count += 1,
                Category::Style => style_count += 1,
                _ => {}
            }
        }

        if security_count > 0 {
            recommendations.push(format!(
                "Address {} security issue(s) immediately",
                security_count
            ));
        }
        if performance_count > 2 {
            recommendations.push(
                "Consider a performance audit - multiple optimization opportunities found"
                    .to_string(),
            );
        }
        if style_count > 5 {
            recommendations
                .push("Consider setting up automated linting to catch style issues".to_string());
        }

        recommendations
    }

    fn deduplicate_comments(comments: &mut Vec<Comment>) {
        comments.sort_by(|a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then(a.line_number.cmp(&b.line_number))
                .then(a.content.cmp(&b.content))
        });
        comments.dedup_by(|a, b| {
            a.file_path == b.file_path && a.line_number == b.line_number && a.content == b.content
        });
    }

    fn sort_by_priority(comments: &mut [Comment]) {
        comments.sort_by_key(|c| {
            let severity_priority = match c.severity {
                Severity::Error => 0,
                Severity::Warning => 1,
                Severity::Info => 2,
                Severity::Suggestion => 3,
            };
            let category_priority = match c.category {
                Category::Security => 0,
                Category::Bug => 1,
                Category::Performance => 2,
                Category::BestPractice => 3,
                Category::Style => 4,
                Category::Documentation => 5,
                Category::Maintainability => 6,
                Category::Testing => 7,
                Category::Architecture => 8,
            };
            (
                severity_priority,
                category_priority,
                c.file_path.clone(),
                c.line_number,
            )
        });
    }
}

pub fn compute_comment_id(file_path: &Path, content: &str, category: &Category) -> String {
    let normalized = normalize_content(content);
    let key = format!("{}|{:?}|{}", file_path.display(), category, normalized);
    let hash = fnv1a64(key.as_bytes());
    format!("cmt_{:016x}", hash)
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn normalize_content(content: &str) -> String {
    let mut normalized = String::new();
    let mut last_space = false;

    for ch in content.chars() {
        let ch = if ch.is_ascii_digit() {
            '#'
        } else {
            ch.to_ascii_lowercase()
        };

        if ch.is_whitespace() {
            if !last_space {
                normalized.push(' ');
                last_space = true;
            }
        } else {
            normalized.push(ch);
            last_space = false;
        }
    }

    normalized.trim().to_string()
}

#[derive(Debug)]
pub struct RawComment {
    pub file_path: PathBuf,
    pub line_number: usize,
    pub content: String,
    pub suggestion: Option<String>,
    pub severity: Option<Severity>,
    pub category: Option<Category>,
    pub confidence: Option<f32>,
    pub fix_effort: Option<FixEffort>,
    pub tags: Vec<String>,
}
