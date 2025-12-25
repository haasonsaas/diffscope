use anyhow::Result;
use ignore::WalkBuilder;
use once_cell::sync::Lazy;
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

        let walker = WalkBuilder::new(repo_root)
            .hidden(true)
            .ignore(true)
            .git_ignore(true)
            .git_exclude(true)
            .git_global(true)
            .build();

        let mut files_seen = 0usize;

        for entry in walker.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if files_seen >= max_files {
                break;
            }

            let relative = path
                .strip_prefix(repo_root)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| path.to_path_buf());
            if should_exclude(&relative) {
                continue;
            }

            let extension = match path.extension().and_then(|ext| ext.to_str()) {
                Some(ext) => ext,
                None => continue,
            };
            let patterns = match patterns_for_extension(extension) {
                Some(patterns) => patterns,
                None => continue,
            };

            let metadata = match fs::metadata(path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.len() as usize > max_bytes {
                continue;
            }

            let bytes = match fs::read(path) {
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
                for pattern in patterns {
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

static SYMBOL_PATTERNS: Lazy<HashMap<&'static str, Vec<Regex>>> = Lazy::new(|| {
    let mut map = HashMap::new();

    map.insert(
        "rs",
        vec![Regex::new(
            r"^\s*(?:pub\s+)?(?:fn|struct|enum|trait|type|impl)\s+([A-Za-z_][A-Za-z0-9_]*)",
        )
        .unwrap()],
    );

    map.insert(
        "py",
        vec![
            Regex::new(r"^\s*def\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            Regex::new(r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
        ],
    );

    map.insert(
        "go",
        vec![
            Regex::new(r"^\s*func\s+(?:\([^)]*\)\s*)?([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            Regex::new(r"^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\s+").unwrap(),
        ],
    );

    map.insert(
        "js",
        vec![
            Regex::new(r"^\s*(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_$][A-Za-z0-9_$]*)")
                .unwrap(),
            Regex::new(r"^\s*(?:export\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            Regex::new(r"^\s*(?:export\s+)?const\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*\(").unwrap(),
        ],
    );

    map.insert(
        "ts",
        vec![
            Regex::new(r"^\s*(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_$][A-Za-z0-9_$]*)")
                .unwrap(),
            Regex::new(r"^\s*(?:export\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            Regex::new(r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            Regex::new(r"^\s*(?:export\s+)?type\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=").unwrap(),
        ],
    );
    map.insert("tsx", map.get("ts").cloned().unwrap_or_default());

    map.insert(
        "java",
        vec![
            Regex::new(r"^\s*(?:public|protected|private)?\s*(?:abstract\s+)?class\s+([A-Za-z_][A-Za-z0-9_]*)")
                .unwrap(),
            Regex::new(r"^\s*(?:public|protected|private)?\s*interface\s+([A-Za-z_][A-Za-z0-9_]*)")
                .unwrap(),
            Regex::new(r"^\s*(?:public|protected|private)?\s*enum\s+([A-Za-z_][A-Za-z0-9_]*)")
                .unwrap(),
        ],
    );

    map.insert(
        "kt",
        vec![
            Regex::new(r"^\s*(?:public|private|protected)?\s*class\s+([A-Za-z_][A-Za-z0-9_]*)")
                .unwrap(),
            Regex::new(r"^\s*(?:public|private|protected)?\s*interface\s+([A-Za-z_][A-Za-z0-9_]*)")
                .unwrap(),
            Regex::new(r"^\s*fun\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
        ],
    );

    map.insert(
        "cs",
        vec![
            Regex::new(r"^\s*(?:public|private|protected|internal)?\s*(?:static\s+)?class\s+([A-Za-z_][A-Za-z0-9_]*)")
                .unwrap(),
            Regex::new(r"^\s*(?:public|private|protected|internal)?\s*interface\s+([A-Za-z_][A-Za-z0-9_]*)")
                .unwrap(),
            Regex::new(r"^\s*(?:public|private|protected|internal)?\s*enum\s+([A-Za-z_][A-Za-z0-9_]*)")
                .unwrap(),
        ],
    );

    map.insert(
        "cpp",
        vec![Regex::new(r"^\s*(?:class|struct)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()],
    );
    map.insert("hpp", map.get("cpp").cloned().unwrap_or_default());
    map.insert("h", map.get("cpp").cloned().unwrap_or_default());
    map.insert("c", map.get("cpp").cloned().unwrap_or_default());

    map.insert(
        "rb",
        vec![
            Regex::new(r"^\s*def\s+([A-Za-z_][A-Za-z0-9_!?=]*)").unwrap(),
            Regex::new(r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            Regex::new(r"^\s*module\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
        ],
    );

    map.insert(
        "php",
        vec![
            Regex::new(r"^\s*function\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            Regex::new(r"^\s*(?:abstract\s+)?class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            Regex::new(r"^\s*interface\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            Regex::new(r"^\s*trait\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
        ],
    );

    map
});

fn patterns_for_extension(ext: &str) -> Option<&'static Vec<Regex>> {
    SYMBOL_PATTERNS.get(ext)
}
