use anyhow::Result;
use async_trait::async_trait;
use crate::core::Comment;

#[async_trait]
pub trait PostProcessor: Send + Sync {
    fn id(&self) -> &str;
    async fn run(&self, comments: Vec<Comment>, repo_path: &str) -> Result<Vec<Comment>>;
}