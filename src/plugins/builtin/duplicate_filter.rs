use crate::core::Comment;
use crate::plugins::PostProcessor;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;

pub struct DuplicateFilter;

impl DuplicateFilter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl PostProcessor for DuplicateFilter {
    fn id(&self) -> &str {
        "duplicate_filter"
    }

    async fn run(&self, mut comments: Vec<Comment>, _repo_path: &str) -> Result<Vec<Comment>> {
        let mut seen = HashSet::new();
        comments.retain(|comment| {
            let key = format!(
                "{}:{}:{}",
                comment.file_path.display(),
                comment.line_number,
                comment.content
            );
            seen.insert(key)
        });

        Ok(comments)
    }
}
