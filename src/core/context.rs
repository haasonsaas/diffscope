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
        let mut chunks = Vec::new();
        
        if symbols.is_empty() {
            return Ok(chunks);
        }
        
        // Search for symbol definitions in the same file first
        let full_path = self.repo_path.join(file_path);
        if full_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&full_path).await {
                let lines: Vec<&str> = content.lines().collect();
                
                for symbol in symbols {
                    // Look for function/class/interface definitions
                    for (line_num, line) in lines.iter().enumerate() {
                        let trimmed = line.trim();
                        if trimmed.contains(&format!("function {}", symbol)) ||
                           trimmed.contains(&format!("class {}", symbol)) ||
                           trimmed.contains(&format!("interface {}", symbol)) ||
                           trimmed.contains(&format!("fn {}", symbol)) ||
                           trimmed.contains(&format!("struct {}", symbol)) ||
                           trimmed.contains(&format!("enum {}", symbol)) ||
                           trimmed.contains(&format!("impl {}", symbol)) {
                            
                            // Extract a few lines around the definition for context
                            let start_line = line_num.saturating_sub(2);
                            let end_line = (line_num + 5).min(lines.len());
                            let definition_content = lines[start_line..end_line].join("\n");
                            
                            chunks.push(LLMContextChunk {
                                file_path: file_path.clone(),
                                content: definition_content,
                                context_type: ContextType::Definition,
                                line_range: Some((start_line + 1, end_line)),
                            });
                        }
                    }
                }
            }
        }
        
        Ok(chunks)
    }
}