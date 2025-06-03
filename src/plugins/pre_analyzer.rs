use anyhow::Result;
use async_trait::async_trait;
use crate::core::{UnifiedDiff, LLMContextChunk};

#[async_trait]
pub trait PreAnalyzer: Send + Sync {
    fn id(&self) -> &str;
    async fn run(&self, diff: &UnifiedDiff, repo_path: &str) -> Result<Vec<LLMContextChunk>>;
}