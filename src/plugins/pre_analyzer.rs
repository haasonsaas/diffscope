use crate::core::{LLMContextChunk, UnifiedDiff};
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait PreAnalyzer: Send + Sync {
    fn id(&self) -> &str;
    async fn run(&self, diff: &UnifiedDiff, repo_path: &str) -> Result<Vec<LLMContextChunk>>;
}
