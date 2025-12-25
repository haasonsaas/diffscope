use anyhow::Result;
use glob::glob;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use crate::core::SymbolIndex;
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

    pub async fn fetch_context_for_file(
        &self,
        file_path: &PathBuf,
        lines: &[(usize, usize)],
    ) -> Result<Vec<LLMContextChunk>> {
        let mut chunks = Vec::new();

        let full_path = self.repo_path.join(file_path);
        if full_path.exists() {
            let content = read_file_lossy(&full_path).await?;
            let file_lines: Vec<&str> = content.lines().collect();
            let merged_ranges = merge_ranges(lines);

            for (start, end) in merged_ranges {
                if file_lines.is_empty() {
                    break;
                }
                let start = start.max(1);
                let end = end.max(start);
                let start_idx = start.saturating_sub(1);
                let end_idx = end.min(file_lines.len());

                if start_idx < file_lines.len() {
                    let chunk_content = truncate_with_notice(
                        file_lines[start_idx..end_idx].join("\n"),
                        MAX_CONTEXT_CHARS,
                    );
                    chunks.push(LLMContextChunk {
                        file_path: file_path.clone(),
                        content: chunk_content,
                        context_type: ContextType::FileContent,
                        line_range: Some((start, end)),
                    });
                }
            }
        }

        Ok(chunks)
    }

    pub async fn fetch_additional_context(
        &self,
        patterns: &[String],
    ) -> Result<Vec<LLMContextChunk>> {
        let mut chunks = Vec::new();
        if patterns.is_empty() {
            return Ok(chunks);
        }

        let mut matched_paths = HashSet::new();
        for pattern in patterns {
            let pattern_path = if Path::new(pattern).is_absolute() {
                pattern.clone()
            } else {
                self.repo_path.join(pattern).to_string_lossy().to_string()
            };

            if let Ok(entries) = glob(&pattern_path) {
                for entry in entries {
                    if let Ok(path) = entry {
                        if path.is_file() {
                            matched_paths.insert(path);
                        }
                    }
                }
            }
        }

        let max_files = 10usize;
        let max_lines = 200usize;

        for path in matched_paths.into_iter().take(max_files) {
            let relative_path = path.strip_prefix(&self.repo_path).unwrap_or(&path);
            let content = read_file_lossy(&path).await?;
            let snippet = content
                .lines()
                .take(max_lines)
                .collect::<Vec<_>>()
                .join("\n");
            let snippet = truncate_with_notice(snippet, MAX_CONTEXT_CHARS);
            if snippet.trim().is_empty() {
                continue;
            }

            chunks.push(LLMContextChunk {
                file_path: relative_path.to_path_buf(),
                content: snippet,
                context_type: ContextType::Reference,
                line_range: None,
            });
        }

        Ok(chunks)
    }

    pub async fn fetch_related_definitions(
        &self,
        file_path: &PathBuf,
        symbols: &[String],
    ) -> Result<Vec<LLMContextChunk>> {
        let mut chunks = Vec::new();

        if symbols.is_empty() {
            return Ok(chunks);
        }

        // Search for symbol definitions in the same file first
        let full_path = self.repo_path.join(file_path);
        if full_path.exists() {
            if let Ok(content) = read_file_lossy(&full_path).await {
                let lines: Vec<&str> = content.lines().collect();

                for symbol in symbols {
                    // Look for function/class/interface definitions
                    for (line_num, line) in lines.iter().enumerate() {
                        let trimmed = line.trim();
                        if trimmed.contains(&format!("function {}", symbol))
                            || trimmed.contains(&format!("class {}", symbol))
                            || trimmed.contains(&format!("interface {}", symbol))
                            || trimmed.contains(&format!("fn {}", symbol))
                            || trimmed.contains(&format!("struct {}", symbol))
                            || trimmed.contains(&format!("enum {}", symbol))
                            || trimmed.contains(&format!("impl {}", symbol))
                        {
                            // Extract a few lines around the definition for context
                            let start_line = line_num.saturating_sub(2);
                            let end_line = (line_num + 5).min(lines.len());
                            let definition_content = truncate_with_notice(
                                lines[start_line..end_line].join("\n"),
                                MAX_CONTEXT_CHARS,
                            );

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

    pub async fn fetch_related_definitions_with_index(
        &self,
        file_path: &PathBuf,
        symbols: &[String],
        index: &SymbolIndex,
        max_locations: usize,
    ) -> Result<Vec<LLMContextChunk>> {
        let mut chunks = Vec::new();

        if symbols.is_empty() {
            return Ok(chunks);
        }

        for symbol in symbols {
            if let Some(locations) = index.lookup(symbol) {
                for location in locations.iter().take(max_locations) {
                    if &location.file_path == file_path {
                        continue;
                    }
                    let snippet = truncate_with_notice(location.snippet.clone(), MAX_CONTEXT_CHARS);
                    chunks.push(LLMContextChunk {
                        file_path: location.file_path.clone(),
                        content: snippet,
                        context_type: ContextType::Definition,
                        line_range: Some(location.line_range),
                    });
                }
            }
        }

        Ok(chunks)
    }
}

fn merge_ranges(lines: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if lines.is_empty() {
        return Vec::new();
    }

    let mut ranges = lines.to_vec();
    ranges.sort_by_key(|(start, _)| *start);

    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        let end = end.max(start);
        if let Some(last) = merged.last_mut() {
            if start <= last.1.saturating_add(1) {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    merged
}

const MAX_CONTEXT_CHARS: usize = 8000;

fn truncate_with_notice(mut content: String, max_chars: usize) -> String {
    if max_chars == 0 || content.len() <= max_chars {
        return content;
    }
    content.truncate(max_chars.saturating_sub(20));
    content.push_str("\n[Truncated]\n");
    content
}

async fn read_file_lossy(path: &Path) -> Result<String> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => Ok(content),
        Err(_) => {
            let bytes = tokio::fs::read(path).await?;
            Ok(String::from_utf8_lossy(&bytes).to_string())
        }
    }
}
