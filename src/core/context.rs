use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMContextChunk {
    pub file_path: PathBuf,
    pub content: String,
    pub context_type: ContextType,
    pub line_range: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextType {
    FileContent,
    Definition,
    Reference,
    Documentation,
}

pub struct ContextFetcher {
    repo_path: PathBuf,
}

impl ContextFetcher {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }

    pub async fn fetch_context_for_file(&self, file_path: &PathBuf, lines: &[(usize, usize)]) -> Result<Vec<LLMContextChunk>> {
        let mut chunks = Vec::new();
        
        let full_path = self.repo_path.join(file_path);
        if full_path.exists() {
            let content = tokio::fs::read_to_string(&full_path).await?;
            
            for (start, end) in lines {
                let file_lines: Vec<&str> = content.lines().collect();
                let start_idx = start.saturating_sub(1);
                let end_idx = (*end).min(file_lines.len());
                
                if start_idx < file_lines.len() {
                    let chunk_content = file_lines[start_idx..end_idx].join("\n");
                    chunks.push(LLMContextChunk {
                        file_path: file_path.clone(),
                        content: chunk_content,
                        context_type: ContextType::FileContent,
                        line_range: Some((*start, *end)),
                    });
                }
            }
        }
        
        Ok(chunks)
    }

    pub async fn fetch_related_definitions(&self, file_path: &PathBuf, symbols: &[String]) -> Result<Vec<LLMContextChunk>> {
        let chunks = Vec::new();
        Ok(chunks)
    }
}