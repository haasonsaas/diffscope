use anyhow::Result;
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SymbolLocation {
    pub file_path: PathBuf,
    pub line_range: (usize, usize),
    pub snippet: String,
}

#[derive(Debug, Default)]
pub struct SymbolIndex {
    symbols: HashMap<String, Vec<SymbolLocation>>,
    files_indexed: usize,
}

impl SymbolIndex {
    pub fn build<F>(
        repo_root: &Path,
        max_files: usize,
        max_bytes: usize,
        max_locations: usize,
        should_exclude: F,
    ) -> Result<Self>
    where
        F: Fn(&PathBuf) -> bool,
    {
        let mut index = SymbolIndex::default();
        if max_files == 0 {
            return Ok(index);
        }

        let patterns = build_symbol_patterns()?;
        let mut stack = vec![repo_root.to_path_buf()];
        let mut files_seen = 0usize;

        while let Some(dir) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if should_skip_dir(&path) {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }

                if files_seen >= max_files {
                    return Ok(index);
                }

                if !is_supported_file(&path) {
                    continue;
                }

                let relative = path
                    .strip_prefix(repo_root)
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|_| path.clone());
                if should_exclude(&relative) {
                    continue;
                }

                let metadata = match fs::metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                };
                if metadata.len() as usize > max_bytes {
                    continue;
                }

                let bytes = match fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(_) => continue,
                };
                if bytes.iter().take(2048).any(|b| *b == 0) {
                    continue;
                }

                let content = String::from_utf8_lossy(&bytes);
                let lines: Vec<&str> = content.lines().collect();
                let mut file_added = false;

                for (idx, line) in lines.iter().enumerate() {
                    for pattern in &patterns {
                        if let Some(caps) = pattern.captures(line) {
                            if let Some(name) = caps.get(1) {
                                let symbol = name.as_str().to_string();
                                if symbol.len() < 2 {
                                    continue;
                                }
                                let entry = index.symbols.entry(symbol).or_default();
                                if entry.len() >= max_locations {
                                    continue;
                                }

                                let start = idx.saturating_sub(2);
                                let end = (idx + 3).min(lines.len().saturating_sub(1));
                                let snippet = lines[start..=end].join("\n");
                                entry.push(SymbolLocation {
                                    file_path: relative.clone(),
                                    line_range: (start + 1, end + 1),
                                    snippet,
                                });
                                file_added = true;
                            }
                        }
                    }
                }

                if file_added {
                    files_seen += 1;
                    index.files_indexed += 1;
                }
            }
        }

        Ok(index)
    }

    pub fn lookup(&self, symbol: &str) -> Option<&Vec<SymbolLocation>> {
        self.symbols.get(symbol)
    }

    pub fn files_indexed(&self) -> usize {
        self.files_indexed
    }

    pub fn symbols_indexed(&self) -> usize {
        self.symbols.len()
    }
}

fn build_symbol_patterns() -> Result<Vec<Regex>> {
    Ok(vec![
        Regex::new(
            r"^\s*(?:pub\s+)?(?:fn|struct|enum|trait|type|impl)\s+([A-Za-z_][A-Za-z0-9_]*)",
        )?,
        Regex::new(r"^\s*(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_$][A-Za-z0-9_$]*)")?,
        Regex::new(r"^\s*(?:export\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)")?,
        Regex::new(r"^\s*def\s+([A-Za-z_][A-Za-z0-9_]*)")?,
        Regex::new(r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)")?,
        Regex::new(r"^\s*func\s+(?:\([^)]*\)\s*)?([A-Za-z_][A-Za-z0-9_]*)")?,
        Regex::new(
            r"^\s*(?:public|private|protected)?\s*(?:static\s+)?(?:class|interface|enum)\s+([A-Za-z_][A-Za-z0-9_]*)",
        )?,
    ])
}

fn is_supported_file(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => matches!(
            ext,
            "rs" | "py"
                | "js"
                | "ts"
                | "tsx"
                | "go"
                | "java"
                | "kt"
                | "cs"
                | "cpp"
                | "c"
                | "h"
                | "hpp"
                | "rb"
                | "php"
        ),
        None => false,
    }
}

fn should_skip_dir(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        matches!(
            name,
            ".git" | "node_modules" | "target" | "dist" | "build" | ".venv" | "venv"
        )
    } else {
        false
    }
}
