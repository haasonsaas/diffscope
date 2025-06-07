use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use crate::core::{UnifiedDiff, LLMContextChunk, Comment};
use crate::plugins::{PreAnalyzer, PostProcessor};

#[async_trait]
#[allow(dead_code)]
pub trait Plugin: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    
    async fn as_pre_analyzer(&self) -> Option<Box<dyn PreAnalyzer>>;
    async fn as_post_processor(&self) -> Option<Box<dyn PostProcessor>>;
}

pub struct PluginManager {
    _plugins: HashMap<String, Arc<dyn Plugin>>,
    pre_analyzers: Vec<Arc<dyn PreAnalyzer>>,
    post_processors: Vec<Arc<dyn PostProcessor>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            pre_analyzers: Vec::new(),
            post_processors: Vec::new(),
        }
    }
    
    pub async fn load_builtin_plugins(&mut self) -> Result<()> {
        self.register_pre_analyzer(Arc::new(crate::plugins::builtin::EslintAnalyzer::new()));
        self.register_pre_analyzer(Arc::new(crate::plugins::builtin::SemgrepAnalyzer::new()));
        self.register_post_processor(Arc::new(crate::plugins::builtin::DuplicateFilter::new()));
        
        Ok(())
    }
    
    pub fn register_pre_analyzer(&mut self, analyzer: Arc<dyn PreAnalyzer>) {
        self.pre_analyzers.push(analyzer);
    }
    
    pub fn register_post_processor(&mut self, processor: Arc<dyn PostProcessor>) {
        self.post_processors.push(processor);
    }
    
    pub async fn run_pre_analyzers(
        &self,
        diff: &UnifiedDiff,
        repo_path: &str,
    ) -> Result<Vec<LLMContextChunk>> {
        let mut all_chunks = Vec::new();
        
        for analyzer in &self.pre_analyzers {
            match analyzer.run(diff, repo_path).await {
                Ok(chunks) => all_chunks.extend(chunks),
                Err(e) => {
                    tracing::warn!("Pre-analyzer {} failed: {}", analyzer.id(), e);
                }
            }
        }
        
        Ok(all_chunks)
    }
    
    pub async fn run_post_processors(
        &self,
        comments: Vec<Comment>,
        repo_path: &str,
    ) -> Result<Vec<Comment>> {
        let mut processed = comments;
        
        for processor in &self.post_processors {
            match processor.run(processed.clone(), repo_path).await {
                Ok(result) => processed = result,
                Err(e) => {
                    tracing::warn!("Post-processor {} failed: {}", processor.id(), e);
                }
            }
        }
        
        Ok(processed)
    }
}