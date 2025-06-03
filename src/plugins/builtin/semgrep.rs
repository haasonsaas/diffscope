use anyhow::Result;
use async_trait::async_trait;
use crate::core::{UnifiedDiff, LLMContextChunk, ContextType};
use crate::plugins::PreAnalyzer;
use std::path::PathBuf;
use std::process::Command;

pub struct SemgrepAnalyzer;

impl SemgrepAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl PreAnalyzer for SemgrepAnalyzer {
    fn id(&self) -> &str {
        "semgrep"
    }
    
    async fn run(&self, diff: &UnifiedDiff, repo_path: &str) -> Result<Vec<LLMContextChunk>> {
        let file_path = PathBuf::from(repo_path).join(&diff.file_path);
        
        let output = Command::new("semgrep")
            .arg("--config=auto")
            .arg("--json")
            .arg("--quiet")
            .arg(file_path.to_string_lossy().as_ref())
            .output();
        
        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if !stdout.trim().is_empty() {
                    Ok(vec![LLMContextChunk {
                        file_path: diff.file_path.clone(),
                        content: format!("Semgrep analysis:\n{}", stdout),
                        context_type: ContextType::Documentation,
                        line_range: None,
                    }])
                } else {
                    Ok(Vec::new())
                }
            }
            Err(_) => {
                Ok(Vec::new())
            }
        }
    }
}