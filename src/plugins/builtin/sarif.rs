use crate::core::comment::{Category, Severity};
use crate::core::{LLMContextChunk, UnifiedDiff};
use crate::plugins::{AnalyzerFinding, PreAnalysis, PreAnalyzer};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::path_utils::normalize_tool_path;

pub struct SarifAnalyzer {
    report_paths: Vec<String>,
}

impl SarifAnalyzer {
    pub fn new(report_paths: Vec<String>) -> Self {
        Self { report_paths }
    }
}

#[async_trait]
impl PreAnalyzer for SarifAnalyzer {
    fn id(&self) -> &str {
        "sarif"
    }

    async fn run(&self, diff: &UnifiedDiff, repo_path: &str) -> Result<PreAnalysis> {
        let results = self
            .run_batch(std::slice::from_ref(diff), repo_path)
            .await?;
        Ok(results.get(&diff.file_path).cloned().unwrap_or_default())
    }

    async fn run_batch(
        &self,
        diffs: &[UnifiedDiff],
        repo_path: &str,
    ) -> Result<HashMap<PathBuf, PreAnalysis>> {
        if self.report_paths.is_empty() {
            return Ok(HashMap::new());
        }

        let repo_root = PathBuf::from(repo_path);
        let changed_files = diffs
            .iter()
            .map(|diff| diff.file_path.clone())
            .collect::<HashSet<_>>();
        let mut analyses: HashMap<PathBuf, PreAnalysis> = HashMap::new();

        for configured_path in &self.report_paths {
            let Some(report_path) = resolve_report_path(&repo_root, configured_path) else {
                tracing::warn!(
                    "Skipping SARIF report outside repository or missing: {}",
                    configured_path
                );
                continue;
            };
            let Ok(payload) = std::fs::read_to_string(&report_path) else {
                tracing::warn!("Unable to read SARIF report: {}", report_path.display());
                continue;
            };
            let report_label = normalize_tool_path(&repo_root, &report_path.to_string_lossy())
                .display()
                .to_string();

            for (file_path, analysis) in
                parse_sarif_analyses(&repo_root, &payload, &changed_files, &report_label)
            {
                analyses.entry(file_path).or_default().extend(analysis);
            }
        }

        Ok(analyses)
    }
}

fn resolve_report_path(repo_root: &Path, configured_path: &str) -> Option<PathBuf> {
    if configured_path.trim().is_empty() {
        return None;
    }

    let repo_root = repo_root.canonicalize().ok()?;
    let candidate = Path::new(configured_path);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        repo_root.join(candidate)
    };
    let candidate = candidate.canonicalize().ok()?;

    if candidate.starts_with(&repo_root) {
        Some(candidate)
    } else {
        None
    }
}

fn parse_sarif_analyses(
    repo_root: &Path,
    payload: &str,
    changed_files: &HashSet<PathBuf>,
    report_label: &str,
) -> HashMap<PathBuf, PreAnalysis> {
    let mut analyses = HashMap::new();
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return analyses;
    };
    let Some(runs) = value.get("runs").and_then(Value::as_array) else {
        return analyses;
    };

    let mut findings_by_file: HashMap<PathBuf, Vec<AnalyzerFinding>> = HashMap::new();
    for run in runs {
        let tool_name = sarif_tool_name(run);
        let rules = sarif_rules(run);
        let Some(results) = run.get("results").and_then(Value::as_array) else {
            continue;
        };

        for result in results {
            let Some(file_path) = sarif_result_path(repo_root, result) else {
                continue;
            };
            if !changed_files.contains(&file_path) {
                continue;
            }

            let rule_id = result
                .get("ruleId")
                .and_then(Value::as_str)
                .map(str::to_string);
            let rule = rule_id.as_ref().and_then(|id| rules.get(id));
            let line_number = sarif_result_line(result).unwrap_or(1);
            let message = sarif_message(result)
                .or_else(|| rule.and_then(|rule| rule.message.clone()))
                .unwrap_or_else(|| "SARIF reported a potential issue".to_string());
            let level = result
                .get("level")
                .and_then(Value::as_str)
                .unwrap_or_else(|| {
                    rule.and_then(|rule| rule.level.as_deref())
                        .unwrap_or("warning")
                });
            let severity = sarif_severity(level, rule.and_then(|rule| rule.security_severity));
            let confidence = sarif_confidence(&severity);
            let category = sarif_category(rule_id.as_deref(), rule, &message);
            let mut metadata = HashMap::new();
            metadata.insert("report".to_string(), report_label.to_string());
            metadata.insert("tool".to_string(), tool_name.clone());
            metadata.insert("level".to_string(), level.to_string());
            if let Some(help_uri) = rule.and_then(|rule| rule.help_uri.clone()) {
                metadata.insert("help_uri".to_string(), help_uri);
            }
            if let Some(security_severity) = rule.and_then(|rule| rule.security_severity) {
                metadata.insert(
                    "security_severity".to_string(),
                    format!("{security_severity:.1}"),
                );
            }

            let tool_tag = format!("sarif-tool:{}", stable_tag_value(&tool_name));
            let level_tag = format!("sarif-level:{}", stable_tag_value(level));
            findings_by_file
                .entry(file_path.clone())
                .or_default()
                .push(AnalyzerFinding {
                    file_path,
                    line_number,
                    content: format!("{tool_name} SARIF finding: {message}"),
                    rule_id,
                    suggestion: None,
                    severity,
                    category,
                    confidence,
                    source: "sarif".to_string(),
                    tags: vec!["sarif".to_string(), tool_tag, level_tag],
                    metadata,
                });
        }
    }

    for (file_path, findings) in findings_by_file {
        let mut analysis = PreAnalysis::default();
        analysis.context_chunks.push(
            LLMContextChunk::documentation(file_path.clone(), build_context_chunk(&findings))
                .with_provenance(crate::core::ContextProvenance::analyzer("sarif")),
        );
        analysis.findings = findings;
        analyses.insert(file_path, analysis);
    }

    analyses
}

#[derive(Debug, Clone, Default)]
struct SarifRule {
    message: Option<String>,
    level: Option<String>,
    help_uri: Option<String>,
    tags: Vec<String>,
    security_severity: Option<f32>,
}

fn sarif_tool_name(run: &Value) -> String {
    run.get("tool")
        .and_then(|tool| tool.get("driver"))
        .and_then(|driver| driver.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("SARIF")
        .to_string()
}

fn sarif_rules(run: &Value) -> HashMap<String, SarifRule> {
    let Some(rules) = run
        .get("tool")
        .and_then(|tool| tool.get("driver"))
        .and_then(|driver| driver.get("rules"))
        .and_then(Value::as_array)
    else {
        return HashMap::new();
    };

    let mut by_id = HashMap::new();
    for rule in rules {
        let Some(id) = rule.get("id").and_then(Value::as_str) else {
            continue;
        };
        by_id.insert(
            id.to_string(),
            SarifRule {
                message: sarif_rule_message(rule),
                level: rule
                    .get("defaultConfiguration")
                    .and_then(|config| config.get("level"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                help_uri: rule
                    .get("helpUri")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                tags: rule
                    .get("properties")
                    .and_then(|properties| properties.get("tags"))
                    .and_then(Value::as_array)
                    .map(|tags| {
                        tags.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                security_severity: rule
                    .get("properties")
                    .and_then(|properties| properties.get("security-severity"))
                    .and_then(parse_sarif_score),
            },
        );
    }

    by_id
}

fn sarif_rule_message(rule: &Value) -> Option<String> {
    for field in ["shortDescription", "fullDescription"] {
        if let Some(text) = rule
            .get(field)
            .and_then(|description| description.get("text"))
            .and_then(Value::as_str)
        {
            return Some(text.to_string());
        }
    }
    None
}

fn parse_sarif_score(value: &Value) -> Option<f32> {
    value
        .as_f64()
        .map(|score| score as f32)
        .or_else(|| value.as_str().and_then(|score| score.parse::<f32>().ok()))
}

fn sarif_result_path(repo_root: &Path, result: &Value) -> Option<PathBuf> {
    result
        .get("locations")?
        .as_array()?
        .first()?
        .get("physicalLocation")?
        .get("artifactLocation")?
        .get("uri")?
        .as_str()
        .map(|uri| normalize_tool_path(repo_root, &decode_sarif_uri(uri)))
}

fn sarif_result_line(result: &Value) -> Option<usize> {
    result
        .get("locations")?
        .as_array()?
        .first()?
        .get("physicalLocation")?
        .get("region")?
        .get("startLine")?
        .as_u64()
        .map(|line| line as usize)
}

fn sarif_message(result: &Value) -> Option<String> {
    result
        .get("message")
        .and_then(|message| {
            message
                .get("text")
                .or_else(|| message.get("markdown"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

fn sarif_severity(level: &str, security_severity: Option<f32>) -> Severity {
    if let Some(security_severity) = security_severity {
        if security_severity >= 7.0 {
            return Severity::Error;
        }
        if security_severity >= 4.0 {
            return Severity::Warning;
        }
    }

    match level.to_ascii_lowercase().as_str() {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => Severity::Info,
    }
}

fn sarif_confidence(severity: &Severity) -> f32 {
    match severity {
        Severity::Error => 0.98,
        Severity::Warning => 0.95,
        Severity::Info | Severity::Suggestion => 0.9,
    }
}

fn sarif_category(rule_id: Option<&str>, rule: Option<&SarifRule>, message: &str) -> Category {
    let mut haystack = String::new();
    if let Some(rule_id) = rule_id {
        haystack.push_str(rule_id);
        haystack.push(' ');
    }
    if let Some(rule) = rule {
        haystack.push_str(&rule.tags.join(" "));
        haystack.push(' ');
    }
    haystack.push_str(message);
    let haystack = haystack.to_ascii_lowercase();

    if haystack.contains("security")
        || haystack.contains("cwe-")
        || haystack.contains("owasp")
        || haystack.contains("vulnerab")
    {
        Category::Security
    } else if haystack.contains("test") {
        Category::Testing
    } else if haystack.contains("performance") {
        Category::Performance
    } else {
        Category::BestPractice
    }
}

fn build_context_chunk(findings: &[AnalyzerFinding]) -> String {
    let details = findings
        .iter()
        .take(20)
        .map(|finding| {
            let rule = finding
                .rule_id
                .as_deref()
                .map(|value| format!(" [{value}]"))
                .unwrap_or_default();
            let tool = finding
                .metadata
                .get("tool")
                .map(String::as_str)
                .unwrap_or("SARIF");
            format!(
                "- {tool} line {}{}: {}",
                finding.line_number, rule, finding.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("SARIF/code-scanning findings:\n{details}")
}

fn decode_sarif_uri(uri: &str) -> String {
    let without_file_scheme = uri.strip_prefix("file://").unwrap_or(uri);
    percent_decode(without_file_scheme)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).to_string()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn stable_tag_value(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed_files(paths: &[&str]) -> HashSet<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn parse_sarif_analyses_groups_changed_file_findings() {
        let payload = r#"{
          "version": "2.1.0",
          "runs": [{
            "tool": {
              "driver": {
                "name": "CodeQL",
                "rules": [{
                  "id": "js/sql-injection",
                  "shortDescription": {"text": "SQL injection"},
                  "helpUri": "https://codeql.github.com/",
                  "defaultConfiguration": {"level": "error"},
                  "properties": {
                    "tags": ["security", "external/cwe/cwe-089"],
                    "security-severity": "8.8"
                  }
                }]
              }
            },
            "results": [
              {
                "ruleId": "js/sql-injection",
                "level": "warning",
                "message": {"text": "User input reaches a SQL query"},
                "locations": [{
                  "physicalLocation": {
                    "artifactLocation": {"uri": "src/app.ts"},
                    "region": {"startLine": 42}
                  }
                }]
              },
              {
                "ruleId": "js/sql-injection",
                "message": {"text": "Unchanged file finding"},
                "locations": [{
                  "physicalLocation": {
                    "artifactLocation": {"uri": "src/other.ts"},
                    "region": {"startLine": 5}
                  }
                }]
              }
            ]
          }]
        }"#;

        let analyses = parse_sarif_analyses(
            Path::new("/repo"),
            payload,
            &changed_files(&["src/app.ts"]),
            "codeql.sarif",
        );

        let analysis = analyses.get(Path::new("src/app.ts")).unwrap();
        assert_eq!(analysis.findings.len(), 1);
        assert!(!analyses.contains_key(Path::new("src/other.ts")));
        assert_eq!(
            analysis.findings[0].rule_id.as_deref(),
            Some("js/sql-injection")
        );
        assert_eq!(analysis.findings[0].severity, Severity::Error);
        assert_eq!(analysis.findings[0].category, Category::Security);
        assert!(analysis.findings[0].tags.contains(&"sarif".to_string()));
        assert!(analysis.context_chunks[0]
            .content
            .contains("SARIF/code-scanning findings"));
    }

    #[test]
    fn parse_sarif_analyses_decodes_file_uris() {
        let payload = r#"{
          "runs": [{
            "tool": {"driver": {"name": "Scanner"}},
            "results": [{
              "ruleId": "style.rule",
              "message": {"text": "Use a safer pattern"},
              "locations": [{
                "physicalLocation": {
                  "artifactLocation": {"uri": "file:///repo/src/my%20file.ts"},
                  "region": {"startLine": 7}
                }
              }]
            }]
          }]
        }"#;

        let analyses = parse_sarif_analyses(
            Path::new("/repo"),
            payload,
            &changed_files(&["src/my file.ts"]),
            "scanner.sarif",
        );

        assert!(analyses.contains_key(Path::new("src/my file.ts")));
    }

    #[test]
    fn resolve_report_path_rejects_parent_traversal() {
        let repo = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        assert!(resolve_report_path(repo.path(), outside.path().to_str().unwrap()).is_none());
    }
}
