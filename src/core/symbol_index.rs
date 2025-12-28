use anyhow::Result;
use ignore::WalkBuilder;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

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

struct LspServerOption {
    command: &'static str,
    program: &'static str,
    extensions: &'static [&'static str],
}

const LSP_DETECT_MAX_FILES: usize = 2000;

const LSP_SERVER_OPTIONS: &[LspServerOption] = &[
    LspServerOption {
        command: "rust-analyzer",
        program: "rust-analyzer",
        extensions: &["rs"],
    },
    LspServerOption {
        command: "typescript-language-server --stdio",
        program: "typescript-language-server",
        extensions: &["ts", "tsx", "js", "jsx"],
    },
    LspServerOption {
        command: "pylsp",
        program: "pylsp",
        extensions: &["py", "pyi"],
    },
    LspServerOption {
        command: "gopls",
        program: "gopls",
        extensions: &["go"],
    },
    LspServerOption {
        command: "jdtls",
        program: "jdtls",
        extensions: &["java"],
    },
    LspServerOption {
        command: "kotlin-lsp",
        program: "kotlin-lsp",
        extensions: &["kt"],
    },
    LspServerOption {
        command: "clangd",
        program: "clangd",
        extensions: &["c", "h", "cc", "cpp", "cxx", "hpp"],
    },
    LspServerOption {
        command: "csharp-ls",
        program: "csharp-ls",
        extensions: &["cs"],
    },
    LspServerOption {
        command: "solargraph stdio",
        program: "solargraph",
        extensions: &["rb"],
    },
    LspServerOption {
        command: "phpactor language-server",
        program: "phpactor",
        extensions: &["php"],
    },
];

impl SymbolIndex {
    pub fn detect_lsp_command<F>(
        repo_root: &Path,
        max_files: usize,
        lsp_languages: &HashMap<String, String>,
        should_exclude: F,
    ) -> Option<String>
    where
        F: Fn(&PathBuf) -> bool,
    {
        if max_files == 0 {
            return None;
        }
        let enabled_extensions = normalized_extension_set(lsp_languages);
        let extension_counts = collect_extension_counts(
            repo_root,
            max_files.min(LSP_DETECT_MAX_FILES),
            &enabled_extensions,
            should_exclude,
        );
        choose_lsp_command(&extension_counts, &enabled_extensions)
    }

    pub fn scan_extension_counts<F>(
        repo_root: &Path,
        max_files: usize,
        should_exclude: F,
    ) -> HashMap<String, usize>
    where
        F: Fn(&PathBuf) -> bool,
    {
        if max_files == 0 {
            return HashMap::new();
        }
        let enabled_extensions = HashSet::new();
        collect_extension_counts(
            repo_root,
            max_files.min(LSP_DETECT_MAX_FILES),
            &enabled_extensions,
            should_exclude,
        )
    }

    pub fn lsp_command_available(command: &str) -> bool {
        let program = match command_program(command) {
            Some(program) => program,
            None => return false,
        };
        is_program_available(&program)
    }

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
            let file_added =
                add_symbols_from_lines(&mut index, &relative, &lines, patterns, max_locations);

            if file_added {
                files_seen += 1;
                index.files_indexed += 1;
            }
        }

        Ok(index)
    }

    pub fn build_with_lsp<F>(
        repo_root: &Path,
        max_files: usize,
        max_bytes: usize,
        max_locations: usize,
        lsp_command: &str,
        lsp_languages: &HashMap<String, String>,
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

        let mut lsp_files: Vec<(PathBuf, String)> = Vec::new();
        let mut other_files = Vec::new();

        for entry in walker.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
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
            if let Some(language_id) = lsp_languages.get(extension) {
                lsp_files.push((relative, language_id.clone()));
            } else if patterns_for_extension(extension).is_some() {
                other_files.push(relative);
            }
        }

        let mut files_seen = 0usize;
        let mut fallback_lsp = false;

        if !lsp_files.is_empty() {
            match LspClient::spawn(lsp_command, repo_root) {
                Ok(mut client) => {
                    for (relative, language_id) in &lsp_files {
                        if files_seen >= max_files {
                            break;
                        }
                        if let Ok(full_path) = repo_root.join(relative).canonicalize() {
                            if let Ok(metadata) = fs::metadata(&full_path) {
                                if metadata.len() as usize > max_bytes {
                                    continue;
                                }
                            }
                            let content = match fs::read_to_string(&full_path) {
                                Ok(content) => content,
                                Err(_) => continue,
                            };
                            if let Ok(file_added) = client.index_file(
                                &mut index,
                                relative,
                                &full_path,
                                &content,
                                language_id,
                                max_locations,
                            ) {
                                if file_added {
                                    files_seen += 1;
                                }
                            }
                        }
                    }
                    let _ = client.shutdown();
                }
                Err(_) => {
                    fallback_lsp = true;
                }
            }
        }

        for relative in other_files.into_iter().chain(
            lsp_files
                .into_iter()
                .filter(|_| fallback_lsp)
                .map(|(path, _)| path),
        ) {
            if files_seen >= max_files {
                break;
            }
            let full_path = repo_root.join(&relative);
            let extension = match full_path.extension().and_then(|ext| ext.to_str()) {
                Some(ext) => ext,
                None => continue,
            };
            let patterns = match patterns_for_extension(extension) {
                Some(patterns) => patterns,
                None => continue,
            };
            let metadata = match fs::metadata(&full_path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.len() as usize > max_bytes {
                continue;
            }
            let bytes = match fs::read(&full_path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            if bytes.iter().take(2048).any(|b| *b == 0) {
                continue;
            }
            let content = String::from_utf8_lossy(&bytes);
            let lines: Vec<&str> = content.lines().collect();
            let file_added =
                add_symbols_from_lines(&mut index, &relative, &lines, patterns, max_locations);
            if file_added {
                files_seen += 1;
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

fn normalized_extension_set(lsp_languages: &HashMap<String, String>) -> HashSet<String> {
    lsp_languages
        .keys()
        .filter(|ext| !ext.trim().is_empty())
        .map(|ext| ext.trim().to_ascii_lowercase())
        .collect()
}

fn collect_extension_counts<F>(
    repo_root: &Path,
    max_files: usize,
    enabled_extensions: &HashSet<String>,
    should_exclude: F,
) -> HashMap<String, usize>
where
    F: Fn(&PathBuf) -> bool,
{
    let walker = WalkBuilder::new(repo_root)
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .build();

    let mut counts = HashMap::new();
    let mut files_seen = 0usize;

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let relative = path
            .strip_prefix(repo_root)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| path.to_path_buf());
        if should_exclude(&relative) {
            continue;
        }

        files_seen += 1;
        if files_seen > max_files {
            break;
        }

        let extension = match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => ext.trim().to_ascii_lowercase(),
            None => continue,
        };
        if extension.is_empty() {
            continue;
        }
        if !enabled_extensions.is_empty() && !enabled_extensions.contains(&extension) {
            continue;
        }

        *counts.entry(extension).or_insert(0) += 1;
    }

    counts
}

fn choose_lsp_command(
    extension_counts: &HashMap<String, usize>,
    enabled_extensions: &HashSet<String>,
) -> Option<String> {
    let mut best_command: Option<&'static str> = None;
    let mut best_score = 0usize;

    for option in LSP_SERVER_OPTIONS {
        if !is_program_available(option.program) {
            continue;
        }

        let score: usize = option
            .extensions
            .iter()
            .filter(|ext| enabled_extensions.is_empty() || enabled_extensions.contains::<str>(ext))
            .filter_map(|ext| extension_counts.get::<str>(ext))
            .sum();

        if score > best_score {
            best_score = score;
            best_command = Some(option.command);
        }
    }

    best_command.map(|command| command.to_string())
}

fn split_command(command: &str) -> Result<Vec<String>> {
    shell_words::split(command).map_err(|err| anyhow::anyhow!(err.to_string()))
}

fn command_program(command: &str) -> Option<String> {
    let parts = split_command(command).ok()?;
    parts.first().cloned()
}

fn is_program_available(program: &str) -> bool {
    if program.trim().is_empty() {
        return false;
    }

    let program_path = Path::new(program);
    if program_path.components().count() > 1 {
        return program_path.is_file();
    }

    let path_var = match env::var_os("PATH") {
        Some(path) => path,
        None => return false,
    };

    for path in env::split_paths(&path_var) {
        if program_exists_in_dir(&path, program) {
            return true;
        }
    }

    false
}

fn program_exists_in_dir(dir: &Path, program: &str) -> bool {
    let candidate = dir.join(program);
    if candidate.is_file() {
        return true;
    }

    if cfg!(windows) && Path::new(program).extension().is_none() {
        let pathext = env::var_os("PATHEXT").unwrap_or_else(|| ".EXE;.CMD;.BAT;.COM".into());
        for ext in pathext.to_string_lossy().split(';') {
            if ext.is_empty() {
                continue;
            }
            let candidate = dir.join(format!("{}{}", program, ext));
            if candidate.is_file() {
                return true;
            }
        }
    }

    false
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

fn add_symbols_from_lines(
    index: &mut SymbolIndex,
    relative: &Path,
    lines: &[&str],
    patterns: &Vec<Regex>,
    max_locations: usize,
) -> bool {
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
                        file_path: relative.to_path_buf(),
                        line_range: (start + 1, end + 1),
                        snippet,
                    });
                    file_added = true;
                }
            }
        }
    }

    file_added
}

struct LspClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    root_uri: String,
}

impl LspClient {
    fn spawn(command: &str, root: &Path) -> Result<Self> {
        let parts = split_command(command)?;
        let (program, args) = parts
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("Empty LSP command"))?;
        let mut cmd = Command::new(program);
        cmd.args(args);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Missing LSP stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Missing LSP stdout"))?;
        let mut client = LspClient {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            root_uri: path_to_uri(root)?,
        };

        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": client.root_uri,
            "capabilities": {
                "textDocument": {
                    "documentSymbol": { "hierarchicalDocumentSymbolSupport": true }
                }
            }
        });
        let _ = client.send_request("initialize", init_params)?;
        client.send_notification("initialized", json!({}))?;

        Ok(client)
    }

    fn index_file(
        &mut self,
        index: &mut SymbolIndex,
        relative: &Path,
        full_path: &Path,
        content: &str,
        language_id: &str,
        max_locations: usize,
    ) -> Result<bool> {
        let uri = path_to_uri(full_path)?;
        self.send_notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": content
                }
            }),
        )?;

        let response = self.send_request(
            "textDocument/documentSymbol",
            json!({
                "textDocument": { "uri": uri }
            }),
        )?;

        let symbols = extract_lsp_symbols(&response);
        if symbols.is_empty() {
            return Ok(false);
        }

        let lines: Vec<&str> = content.lines().collect();
        let mut file_added = false;

        for symbol in symbols {
            let entry = index.symbols.entry(symbol.name.clone()).or_default();
            if entry.len() >= max_locations {
                continue;
            }

            let start = symbol.range.0.max(1);
            let end = symbol.range.1.max(start);
            let start_idx = start.saturating_sub(3);
            let end_idx = end.saturating_add(2).min(lines.len());
            let snippet = if start_idx < end_idx && start_idx < lines.len() {
                lines[start_idx..end_idx].join("\n")
            } else {
                String::new()
            };

            entry.push(SymbolLocation {
                file_path: relative.to_path_buf(),
                line_range: (start, end),
                snippet,
            });
            file_added = true;
        }

        Ok(file_added)
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.send_message(&message)?;

        loop {
            let response = self.read_message()?;
            if response.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(error) = response.get("error") {
                    return Err(anyhow::anyhow!("LSP error: {}", error));
                }
                return Ok(response.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.send_message(&message)
    }

    fn send_message(&mut self, message: &Value) -> Result<()> {
        let body = serde_json::to_vec(message)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes())?;
        self.stdin.write_all(&body)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_message(&mut self) -> Result<Value> {
        let mut content_length = None;
        loop {
            let mut header = String::new();
            let bytes = self.stdout.read_line(&mut header)?;
            if bytes == 0 {
                return Err(anyhow::anyhow!("LSP closed connection"));
            }
            let header_trimmed = header.trim();
            if header_trimmed.is_empty() {
                break;
            }
            if let Some(rest) = header_trimmed.strip_prefix("Content-Length:") {
                content_length = rest.trim().parse::<usize>().ok();
            }
        }

        let length = content_length.ok_or_else(|| anyhow::anyhow!("Missing Content-Length"))?;
        let mut buffer = vec![0u8; length];
        self.stdout.read_exact(&mut buffer)?;
        let value: Value = serde_json::from_slice(&buffer)?;
        Ok(value)
    }

    fn shutdown(&mut self) -> Result<()> {
        let _ = self.send_request("shutdown", json!({}));
        let _ = self.send_notification("exit", json!({}));
        let _ = self.child.kill();
        Ok(())
    }
}

#[derive(Debug)]
struct LspSymbol {
    name: String,
    range: (usize, usize),
}

fn extract_lsp_symbols(result: &Value) -> Vec<LspSymbol> {
    let mut symbols = Vec::new();
    if let Some(array) = result.as_array() {
        for entry in array {
            collect_lsp_symbol(entry, &mut symbols);
        }
    }
    symbols
}

fn collect_lsp_symbol(value: &Value, symbols: &mut Vec<LspSymbol>) {
    if let Some(obj) = value.as_object() {
        if let (Some(name), Some(range)) = (
            obj.get("name").and_then(|v| v.as_str()),
            extract_range(obj.get("selectionRange").or_else(|| obj.get("range"))),
        ) {
            symbols.push(LspSymbol {
                name: name.to_string(),
                range,
            });
        }

        if let Some(location) = obj.get("location") {
            if let (Some(name), Some(range)) = (
                obj.get("name").and_then(|v| v.as_str()),
                extract_range(location.get("range")),
            ) {
                symbols.push(LspSymbol {
                    name: name.to_string(),
                    range,
                });
            }
        }

        if let Some(children) = obj.get("children") {
            if let Some(child_array) = children.as_array() {
                for child in child_array {
                    collect_lsp_symbol(child, symbols);
                }
            }
        }
    }
}

fn extract_range(value: Option<&Value>) -> Option<(usize, usize)> {
    let range = value?.as_object()?;
    let start = range.get("start")?.as_object()?;
    let end = range.get("end")?.as_object()?;
    let start_line = start.get("line")?.as_u64()? as usize + 1;
    let end_line = end.get("line")?.as_u64()? as usize + 1;
    Some((start_line, end_line.max(start_line)))
}

fn path_to_uri(path: &Path) -> Result<String> {
    let absolute = path.canonicalize()?;
    let path_str = absolute.to_string_lossy().replace('\\', "/");
    let encoded = path_str
        .split('/')
        .map(url_encode)
        .collect::<Vec<_>>()
        .join("/");
    Ok(format!("file://{}", encoded))
}

fn url_encode(segment: &str) -> String {
    let mut out = String::new();
    for ch in segment.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' || ch == '~' {
            out.push(ch);
        } else {
            out.push_str(&format!("%{:02X}", ch as u32));
        }
    }
    out
}
