use anyhow::Result;
use glob::glob;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use crate::core::function_chunker::find_enclosing_boundary_line;
use crate::core::{ContextProvenance, SymbolContextRetriever, SymbolIndex, SymbolRetrievalPolicy};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMContextChunk {
    pub file_path: PathBuf,
    pub content: String,
    pub context_type: ContextType,
    pub line_range: Option<(usize, usize)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<ContextProvenance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContextType {
    FileContent,
    Definition,
    Reference,
    Documentation,
}

impl LLMContextChunk {
    pub fn new(
        file_path: impl Into<PathBuf>,
        content: impl Into<String>,
        context_type: ContextType,
    ) -> Self {
        Self {
            file_path: file_path.into(),
            content: content.into(),
            context_type,
            line_range: None,
            provenance: None,
        }
    }

    pub fn file_content(file_path: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        Self::new(file_path, content, ContextType::FileContent)
    }

    pub fn definition(file_path: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        Self::new(file_path, content, ContextType::Definition)
    }

    pub fn reference(file_path: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        Self::new(file_path, content, ContextType::Reference)
    }

    pub fn documentation(file_path: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        Self::new(file_path, content, ContextType::Documentation)
    }

    pub fn with_line_range(mut self, line_range: (usize, usize)) -> Self {
        self.line_range = Some(line_range);
        self
    }

    pub fn with_provenance(mut self, provenance: ContextProvenance) -> Self {
        self.provenance = Some(provenance);
        self
    }

    pub fn provenance_label(&self) -> Option<String> {
        self.provenance.as_ref().map(ToString::to_string)
    }
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
        file_path: &Path,
        lines: &[(usize, usize)],
    ) -> Result<Vec<LLMContextChunk>> {
        let mut chunks = Vec::new();

        let Some(full_path) = resolve_inside_base(&self.repo_path, file_path) else {
            return Ok(chunks);
        };
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

                // Dynamic context: expand start to enclosing function boundary
                let expanded_start = find_enclosing_boundary_line(&content, file_path, start, 10)
                    .filter(|&boundary| boundary >= start.saturating_sub(10))
                    .unwrap_or_else(|| start.saturating_sub(5)); // fallback: 5 lines before
                let expanded_start = expanded_start.max(1);

                // Asymmetric: less context after (1 extra line)
                let expanded_end = (end + 1).min(file_lines.len());

                let start_idx = expanded_start.saturating_sub(1);
                let end_idx = expanded_end.min(file_lines.len());

                if start_idx < file_lines.len() {
                    let chunk_content = truncate_with_notice(
                        file_lines[start_idx..end_idx].join("\n"),
                        MAX_CONTEXT_CHARS,
                    );
                    chunks.push(
                        LLMContextChunk::file_content(file_path.to_path_buf(), chunk_content)
                            .with_line_range((expanded_start, expanded_end)),
                    );
                }
            }
        }

        Ok(chunks)
    }

    pub async fn fetch_additional_context(
        &self,
        patterns: &[String],
    ) -> Result<Vec<LLMContextChunk>> {
        self.fetch_additional_context_from_base(&self.repo_path, patterns, 10, 200)
            .await
    }

    pub async fn fetch_additional_context_from_base(
        &self,
        base_path: &Path,
        patterns: &[String],
        max_files: usize,
        max_lines: usize,
    ) -> Result<Vec<LLMContextChunk>> {
        let mut chunks = Vec::new();
        if patterns.is_empty() {
            return Ok(chunks);
        }

        let base_path = canonical_base(base_path)?;
        let mut matched_paths = BTreeSet::new();
        for pattern in patterns {
            let pattern_path = if Path::new(pattern).is_absolute() {
                pattern.to_string()
            } else {
                base_path.join(pattern).to_string_lossy().to_string()
            };

            if let Ok(entries) = glob(&pattern_path) {
                for path in entries.flatten() {
                    let Some(safe_path) = canonical_file_inside_base(&base_path, &path) else {
                        continue;
                    };
                    if safe_path.is_file() {
                        matched_paths.insert(safe_path);
                    }
                }
            }
        }

        for path in matched_paths.into_iter().take(max_files) {
            let relative_path = path.strip_prefix(&base_path).unwrap_or(&path);
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

            chunks.push(LLMContextChunk::reference(
                relative_path.to_path_buf(),
                snippet,
            ));
        }

        Ok(chunks)
    }

    pub async fn fetch_related_definitions(
        &self,
        file_path: &Path,
        symbols: &[String],
    ) -> Result<Vec<LLMContextChunk>> {
        let mut chunks = Vec::new();

        if symbols.is_empty() {
            return Ok(chunks);
        }

        // Search for symbol definitions in the same file first
        let Some(full_path) = resolve_inside_base(&self.repo_path, file_path) else {
            return Ok(chunks);
        };
        if full_path.exists() {
            if let Ok(content) = read_file_lossy(&full_path).await {
                let lines: Vec<&str> = content.lines().collect();

                for symbol in symbols {
                    // Look for function/class/interface definitions
                    for (line_num, line) in lines.iter().enumerate() {
                        let trimmed = line.trim();
                        if trimmed.contains(&format!("function {symbol}"))
                            || trimmed.contains(&format!("class {symbol}"))
                            || trimmed.contains(&format!("interface {symbol}"))
                            || trimmed.contains(&format!("fn {symbol}"))
                            || trimmed.contains(&format!("struct {symbol}"))
                            || trimmed.contains(&format!("enum {symbol}"))
                            || trimmed.contains(&format!("impl {symbol}"))
                        {
                            // Extract a few lines around the definition for context
                            let start_line = line_num.saturating_sub(2);
                            let end_line = (line_num + 5).min(lines.len());
                            let definition_content = truncate_with_notice(
                                lines[start_line..end_line].join("\n"),
                                MAX_CONTEXT_CHARS,
                            );

                            chunks.push(
                                LLMContextChunk::definition(
                                    file_path.to_path_buf(),
                                    definition_content,
                                )
                                .with_line_range((start_line + 1, end_line)),
                            );
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
        graph_hops: usize,
        graph_max_files: usize,
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
                    let mut chunk =
                        LLMContextChunk::definition(location.file_path.clone(), snippet)
                            .with_line_range(location.line_range);
                    if let Some(provenance) = location.provenance.clone() {
                        chunk = chunk.with_provenance(provenance);
                    }
                    chunks.push(chunk);
                }
            }
        }

        let retriever = SymbolContextRetriever::new(
            index,
            SymbolRetrievalPolicy::new(max_locations, graph_hops, graph_max_files),
        );
        let related_locations = retriever.related_symbol_locations(file_path, symbols);

        for location in related_locations.definition_locations {
            if &location.file_path == file_path {
                continue;
            }
            let snippet = truncate_with_notice(location.snippet, MAX_CONTEXT_CHARS);
            let mut chunk = LLMContextChunk::definition(location.file_path, snippet)
                .with_line_range(location.line_range);
            if let Some(provenance) = location.provenance {
                chunk = chunk.with_provenance(provenance);
            }
            chunks.push(chunk);
        }

        for location in related_locations.reference_locations {
            if &location.file_path == file_path {
                continue;
            }
            let snippet = truncate_with_notice(location.snippet, MAX_CONTEXT_CHARS);
            let mut chunk = LLMContextChunk::reference(location.file_path, snippet)
                .with_line_range(location.line_range);
            if let Some(provenance) = location.provenance {
                chunk = chunk.with_provenance(provenance);
            }
            chunks.push(chunk);
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

fn canonical_base(base_path: &Path) -> Result<PathBuf> {
    Ok(base_path.canonicalize()?)
}

fn canonical_file_inside_base(base_path: &Path, path: &Path) -> Option<PathBuf> {
    let canonical_path = path.canonicalize().ok()?;
    if canonical_path.starts_with(base_path) {
        Some(canonical_path)
    } else {
        None
    }
}

fn resolve_inside_base(base_path: &Path, path: &Path) -> Option<PathBuf> {
    let base_path = canonical_base(base_path).ok()?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_path.join(path)
    };
    canonical_file_inside_base(&base_path, &candidate)
}

fn truncate_with_notice(mut content: String, max_chars: usize) -> String {
    if max_chars == 0 || content.len() <= max_chars {
        return content;
    }
    let mut end = max_chars.saturating_sub(20);
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    content.truncate(end);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_with_notice_utf8_safety() {
        // '€' is 3 bytes in UTF-8. With 5 euros = 15 bytes,
        // max_chars=10 means truncate at 10 - 20 = 0 (saturating), but
        // let's use a value where the truncation point lands mid-character.
        let content = "€€€€€€€€€€".to_string(); // 10 euros = 30 bytes
                                                // max_chars=25: truncate at 25-20=5, but byte 5 is mid-char (€ boundaries: 0,3,6,9,...)
                                                // This should NOT panic
        let result = truncate_with_notice(content, 25);
        assert!(result.contains("[Truncated]"));
        // Verify the result is valid UTF-8 (it is since it's a String, but
        // the point is truncate() would have panicked)
        assert!(!result.is_empty());
    }

    #[test]
    fn test_truncate_with_notice_ascii() {
        let content = "hello world, this is a long string".to_string();
        let result = truncate_with_notice(content, 30);
        assert!(result.contains("[Truncated]"));
    }

    #[test]
    fn test_truncate_with_notice_no_truncation() {
        let content = "short".to_string();
        let result = truncate_with_notice(content, 100);
        assert_eq!(result, "short");
        assert!(!result.contains("[Truncated]"));
    }

    #[test]
    fn test_merge_ranges_basic() {
        let ranges = vec![(1, 5), (3, 8), (10, 15)];
        let merged = merge_ranges(&ranges);
        assert_eq!(merged, vec![(1, 8), (10, 15)]);
    }

    #[test]
    fn test_merge_ranges_empty() {
        let merged = merge_ranges(&[]);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_ranges_adjacent() {
        let ranges = vec![(1, 5), (6, 10)];
        let merged = merge_ranges(&ranges);
        assert_eq!(merged, vec![(1, 10)]);
    }

    #[tokio::test]
    async fn test_fetch_context_expands_to_function_boundary() {
        // Create a temp file with a function
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        let content = "use std::io;\n\npub fn process(x: i32) -> bool {\n    let y = x + 1;\n    y > 0\n}\n\npub fn other() {\n    println!(\"hi\");\n}\n";
        std::fs::write(&file_path, content).unwrap();

        let fetcher = ContextFetcher::new(dir.path().to_path_buf());
        let relative = PathBuf::from("test.rs");
        // Request context for line 4-5 (inside process function)
        let chunks = fetcher
            .fetch_context_for_file(&relative, &[(4, 5)])
            .await
            .unwrap();

        assert!(!chunks.is_empty());
        // Should expand to include the function signature (line 3)
        let chunk = &chunks[0];
        assert!(chunk.content.contains("pub fn process"));
    }

    #[tokio::test]
    async fn test_fetch_context_rejects_parent_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(dir.path().join("outside.rs"), "pub fn secret() {}\n").unwrap();

        let fetcher = ContextFetcher::new(repo);
        let chunks = fetcher
            .fetch_context_for_file(&PathBuf::from("../outside.rs"), &[(1, 1)])
            .await
            .unwrap();

        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn test_fetch_additional_context_rejects_absolute_and_parent_traversal_outside_base() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let docs = repo.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("review.md"), "safe note").unwrap();
        std::fs::write(dir.path().join("secret.md"), "do not read").unwrap();

        let fetcher = ContextFetcher::new(repo.clone());
        let chunks = fetcher
            .fetch_additional_context(&[
                "docs/*.md".to_string(),
                "../*.md".to_string(),
                dir.path().join("secret.md").display().to_string(),
            ])
            .await
            .unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].file_path, PathBuf::from("docs/review.md"));
        assert!(chunks[0].content.contains("safe note"));
        assert!(!chunks[0].content.contains("do not read"));
    }

    #[tokio::test]
    async fn test_fetch_related_definitions_with_index_uses_symbol_graph_neighbors() {
        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();

        std::fs::write(
            src_dir.join("auth.rs"),
            "pub fn validate_token(token: &str) -> bool {\n    token.len() > 10\n}\n",
        )
        .unwrap();
        std::fs::write(
            src_dir.join("handler.rs"),
            "use crate::auth::validate_token;\n\npub fn handle_request(token: &str) -> bool {\n    validate_token(token)\n}\n",
        )
        .unwrap();

        let index = SymbolIndex::build(dir.path(), 20, 200_000, 10, |_| false).unwrap();
        let fetcher = ContextFetcher::new(dir.path().to_path_buf());

        let chunks = fetcher
            .fetch_related_definitions_with_index(
                &PathBuf::from("src/handler.rs"),
                &["handle_request".to_string()],
                &index,
                10,
                2,
                5,
            )
            .await
            .unwrap();

        let graph_chunk = chunks
            .iter()
            .find(|chunk| chunk.file_path == std::path::Path::new("src/auth.rs"))
            .expect("expected graph-related auth context");

        assert_eq!(graph_chunk.context_type, ContextType::Definition);
        assert!(graph_chunk.content.contains("validate_token"));
        assert!(graph_chunk
            .provenance_label()
            .as_deref()
            .unwrap_or_default()
            .contains("symbol graph"));
        assert!(graph_chunk
            .provenance_label()
            .as_deref()
            .unwrap_or_default()
            .contains("calls"));
    }

    #[tokio::test]
    async fn test_fetch_related_definitions_with_index_surfaces_trait_impl_contract_edges() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(
            dir.path().join("routes.rs"),
            "use crate::request::Request;\nuse crate::search::QueryRunner;\n\npub fn get_profile(runner: &dyn QueryRunner, request: &Request) -> String {\n    runner.find_user(request.name())\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("search.rs"),
            "pub trait QueryRunner {\n    fn find_user(&self, name: &str) -> String;\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("db.rs"),
            "use crate::search::QueryRunner;\n\npub struct PostgresQueryRunner;\n\nimpl QueryRunner for PostgresQueryRunner {\n    fn find_user(&self, name: &str) -> String {\n        format!(\"SELECT * FROM users WHERE name = '{}'\", name)\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("request.rs"),
            "pub struct Request {\n    name: String,\n}\n\nimpl Request {\n    pub fn name(&self) -> &str {\n        &self.name\n    }\n}\n",
        )
        .unwrap();

        let index = SymbolIndex::build(dir.path(), 20, 200_000, 10, |_| false).unwrap();
        let fetcher = ContextFetcher::new(dir.path().to_path_buf());

        let chunks = fetcher
            .fetch_related_definitions_with_index(
                &PathBuf::from("routes.rs"),
                &["get_profile".to_string()],
                &index,
                10,
                2,
                8,
            )
            .await
            .unwrap();

        let impl_chunk = chunks
            .iter()
            .find(|chunk| {
                chunk.file_path == Path::new("db.rs") && chunk.content.contains("find_user")
            })
            .expect("expected trait implementation context for db.rs");

        assert_eq!(impl_chunk.context_type, ContextType::Definition);
        assert!(impl_chunk.content.contains("SELECT * FROM users"));
        assert!(impl_chunk
            .provenance_label()
            .as_deref()
            .unwrap_or_default()
            .contains("calls"));
    }
}
