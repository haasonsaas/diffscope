use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub file_path: PathBuf,
    pub line_number: usize,
    pub content: String,
    pub severity: Severity,
    pub category: Category,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Suggestion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Category {
    Bug,
    Security,
    Performance,
    Style,
    Documentation,
    BestPractice,
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

    fn process_raw_comment(raw: RawComment) -> Result<Option<Comment>> {
        let severity = Self::determine_severity(&raw.content);
        let category = Self::determine_category(&raw.content);
        
        Ok(Some(Comment {
            file_path: raw.file_path,
            line_number: raw.line_number,
            content: raw.content,
            severity,
            category,
            suggestion: raw.suggestion,
        }))
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
        if lower.contains("security") || lower.contains("vulnerability") {
            Category::Security
        } else if lower.contains("performance") || lower.contains("optimization") {
            Category::Performance
        } else if lower.contains("bug") || lower.contains("fix") {
            Category::Bug
        } else if lower.contains("style") || lower.contains("format") {
            Category::Style
        } else if lower.contains("doc") || lower.contains("comment") {
            Category::Documentation
        } else {
            Category::BestPractice
        }
    }

    fn deduplicate_comments(comments: &mut Vec<Comment>) {
        comments.sort_by(|a, b| {
            a.file_path.cmp(&b.file_path)
                .then(a.line_number.cmp(&b.line_number))
                .then(a.content.cmp(&b.content))
        });
        comments.dedup_by(|a, b| {
            a.file_path == b.file_path && 
            a.line_number == b.line_number && 
            a.content == b.content
        });
    }

    fn sort_by_priority(comments: &mut Vec<Comment>) {
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
            };
            (severity_priority, category_priority, c.file_path.clone(), c.line_number)
        });
    }
}

#[derive(Debug)]
pub struct RawComment {
    pub file_path: PathBuf,
    pub line_number: usize,
    pub content: String,
    pub suggestion: Option<String>,
}