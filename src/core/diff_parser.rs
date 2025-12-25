use anyhow::Result;
use serde::{Deserialize, Serialize};
use similar::TextDiff;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedDiff {
    pub file_path: PathBuf,
    pub old_content: Option<String>,
    pub new_content: Option<String>,
    pub hunks: Vec<DiffHunk>,
    pub is_binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub context: String,
    pub changes: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub old_line_no: Option<usize>,
    pub new_line_no: Option<usize>,
    pub change_type: ChangeType,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChangeType {
    Added,
    Removed,
    Context,
}

pub struct DiffParser;

impl DiffParser {
    pub fn parse_unified_diff(diff_content: &str) -> Result<Vec<UnifiedDiff>> {
        let mut diffs = Vec::new();
        let lines: Vec<&str> = diff_content.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            if lines[i].starts_with("diff --git") {
                let diff = Self::parse_single_file_diff(&lines, &mut i)?;
                diffs.push(diff);
            } else if lines[i].starts_with("--- ")
                && i + 1 < lines.len()
                && lines[i + 1].starts_with("+++ ")
            {
                let diff = Self::parse_simple_file_diff(&lines, &mut i)?;
                diffs.push(diff);
            } else {
                i += 1;
            }
        }

        Ok(diffs)
    }

    pub fn parse_text_diff(old_content: &str, new_content: &str, file_path: PathBuf) -> Result<UnifiedDiff> {
        let diff = TextDiff::from_lines(old_content, new_content);
        let mut hunks = Vec::new();

        for group in diff.grouped_ops(3) {
            let mut hunk_lines = Vec::new();
            let mut old_start = None;
            let mut new_start = None;
            let mut old_count = 0;
            let mut new_count = 0;

            for op in group {
                match op.tag() {
                    similar::DiffTag::Delete => {
                        for old_idx in op.old_range() {
                            if old_start.is_none() {
                                old_start = Some(old_idx + 1);
                            }
                            old_count += 1;
                            hunk_lines.push(DiffLine {
                                old_line_no: Some(old_idx + 1),
                                new_line_no: None,
                                change_type: ChangeType::Removed,
                                content: diff.old_slices()[old_idx].to_string(),
                            });
                        }
                    }
                    similar::DiffTag::Insert => {
                        for new_idx in op.new_range() {
                            if new_start.is_none() {
                                new_start = Some(new_idx + 1);
                            }
                            new_count += 1;
                            hunk_lines.push(DiffLine {
                                old_line_no: None,
                                new_line_no: Some(new_idx + 1),
                                change_type: ChangeType::Added,
                                content: diff.new_slices()[new_idx].to_string(),
                            });
                        }
                    }
                    similar::DiffTag::Equal => {
                        for (old_idx, new_idx) in op.old_range().zip(op.new_range()) {
                            if old_start.is_none() {
                                old_start = Some(old_idx + 1);
                            }
                            if new_start.is_none() {
                                new_start = Some(new_idx + 1);
                            }
                            old_count += 1;
                            new_count += 1;
                            hunk_lines.push(DiffLine {
                                old_line_no: Some(old_idx + 1),
                                new_line_no: Some(new_idx + 1),
                                change_type: ChangeType::Context,
                                content: diff.old_slices()[old_idx].to_string(),
                            });
                        }
                    }
                    similar::DiffTag::Replace => {
                        for old_idx in op.old_range() {
                            if old_start.is_none() {
                                old_start = Some(old_idx + 1);
                            }
                            old_count += 1;
                            hunk_lines.push(DiffLine {
                                old_line_no: Some(old_idx + 1),
                                new_line_no: None,
                                change_type: ChangeType::Removed,
                                content: diff.old_slices()[old_idx].to_string(),
                            });
                        }
                        for new_idx in op.new_range() {
                            if new_start.is_none() {
                                new_start = Some(new_idx + 1);
                            }
                            new_count += 1;
                            hunk_lines.push(DiffLine {
                                old_line_no: None,
                                new_line_no: Some(new_idx + 1),
                                change_type: ChangeType::Added,
                                content: diff.new_slices()[new_idx].to_string(),
                            });
                        }
                    }
                }
            }

            if !hunk_lines.is_empty() {
                hunks.push(DiffHunk {
                    old_start: old_start.unwrap_or(1),
                    old_lines: old_count,
                    new_start: new_start.unwrap_or(1),
                    new_lines: new_count,
                    context: format!("@@ -{},{} +{},{} @@", 
                        old_start.unwrap_or(1), old_count,
                        new_start.unwrap_or(1), new_count),
                    changes: hunk_lines,
                });
            }
        }

        Ok(UnifiedDiff {
            file_path,
            old_content: Some(old_content.to_string()),
            new_content: Some(new_content.to_string()),
            hunks,
            is_binary: false,
        })
    }

    fn parse_single_file_diff(lines: &[&str], i: &mut usize) -> Result<UnifiedDiff> {
        let file_line = lines[*i];
        let file_path = Self::extract_file_path(file_line)?;
        *i += 1;

        let mut is_binary = false;
        while *i < lines.len() && !lines[*i].starts_with("@@") && !lines[*i].starts_with("diff --git") {
            if lines[*i].starts_with("Binary files") || lines[*i].starts_with("GIT binary patch") {
                is_binary = true;
            }
            *i += 1;
        }

        let mut hunks = Vec::new();
        
        while *i < lines.len() && lines[*i].starts_with("@@") {
            let hunk = Self::parse_hunk(lines, i)?;
            hunks.push(hunk);
        }

        Ok(UnifiedDiff {
            file_path: PathBuf::from(file_path),
            old_content: None,
            new_content: None,
            hunks,
            is_binary,
        })
    }

    fn parse_simple_file_diff(lines: &[&str], i: &mut usize) -> Result<UnifiedDiff> {
        let old_line = lines[*i];
        let new_line = lines.get(*i + 1).unwrap_or(&"");

        let old_path = Self::extract_path_from_header(old_line, "--- ")?;
        let new_path = Self::extract_path_from_header(new_line, "+++ ")?;

        let file_path = if new_path != "/dev/null" {
            new_path
        } else {
            old_path
        };

        *i += 2;

        let mut hunks = Vec::new();
        let mut is_binary = false;

        while *i < lines.len()
            && !lines[*i].starts_with("diff --git")
            && !(lines[*i].starts_with("--- ")
                && *i + 1 < lines.len()
                && lines[*i + 1].starts_with("+++ "))
        {
            if lines[*i].starts_with("Binary files") || lines[*i].starts_with("GIT binary patch") {
                is_binary = true;
            }
            if lines[*i].starts_with("@@") {
                let hunk = Self::parse_hunk(lines, i)?;
                hunks.push(hunk);
            } else {
                *i += 1;
            }
        }

        Ok(UnifiedDiff {
            file_path: PathBuf::from(file_path),
            old_content: None,
            new_content: None,
            hunks,
            is_binary,
        })
    }

    fn extract_file_path(line: &str) -> Result<String> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            Ok(parts[2].trim_start_matches("a/").to_string())
        } else {
            anyhow::bail!("Invalid diff header: {}", line)
        }
    }

    fn extract_path_from_header(line: &str, prefix: &str) -> Result<String> {
        let raw = line
            .strip_prefix(prefix)
            .ok_or_else(|| anyhow::anyhow!("Invalid file header: {}", line))?
            .trim();
        let path = raw.split_whitespace().next().unwrap_or(raw);
        Ok(path.trim_start_matches("a/").trim_start_matches("b/").to_string())
    }

    fn parse_hunk(lines: &[&str], i: &mut usize) -> Result<DiffHunk> {
        let header = lines[*i];
        let (old_start, old_lines, new_start, new_lines) = Self::parse_hunk_header(header)?;
        *i += 1;

        let mut changes = Vec::new();
        let mut old_line = old_start;
        let mut new_line = new_start;

        while *i < lines.len()
            && !lines[*i].starts_with("@@")
            && !lines[*i].starts_with("diff --git")
            && !lines[*i].starts_with("--- ")
            && !lines[*i].starts_with("+++ ")
        {
            let line = lines[*i];
            if line.is_empty() {
                *i += 1;
                continue;
            }

            let (change_type, content) = match line.chars().next() {
                Some('+') => (ChangeType::Added, &line[1..]),
                Some('-') => (ChangeType::Removed, &line[1..]),
                Some(' ') => (ChangeType::Context, &line[1..]),
                _ => (ChangeType::Context, line),
            };

            let diff_line = match change_type {
                ChangeType::Added => {
                    let line_no = new_line;
                    new_line += 1;
                    DiffLine {
                        old_line_no: None,
                        new_line_no: Some(line_no),
                        change_type,
                        content: content.to_string(),
                    }
                }
                ChangeType::Removed => {
                    let line_no = old_line;
                    old_line += 1;
                    DiffLine {
                        old_line_no: Some(line_no),
                        new_line_no: None,
                        change_type,
                        content: content.to_string(),
                    }
                }
                ChangeType::Context => {
                    let old_no = old_line;
                    let new_no = new_line;
                    old_line += 1;
                    new_line += 1;
                    DiffLine {
                        old_line_no: Some(old_no),
                        new_line_no: Some(new_no),
                        change_type,
                        content: content.to_string(),
                    }
                }
            };

            changes.push(diff_line);
            *i += 1;
        }

        Ok(DiffHunk {
            old_start,
            old_lines,
            new_start,
            new_lines,
            context: header.to_string(),
            changes,
        })
    }

    fn parse_hunk_header(header: &str) -> Result<(usize, usize, usize, usize)> {
        let re = regex::Regex::new(r"@@ -(\d+),?(\d*) \+(\d+),?(\d*) @@")?;
        let caps = re.captures(header)
            .ok_or_else(|| anyhow::anyhow!("Invalid hunk header: {}", header))?;

        let old_start = caps.get(1).unwrap().as_str().parse()?;
        let old_lines = caps.get(2).map_or(1, |m| m.as_str().parse().unwrap_or(1));
        let new_start = caps.get(3).unwrap().as_str().parse()?;
        let new_lines = caps.get(4).map_or(1, |m| m.as_str().parse().unwrap_or(1));

        Ok((old_start, old_lines, new_start, new_lines))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_diff() {
        let old = "line1\nline2\nline3";
        let new = "line1\nmodified\nline3\nline4";
        
        let diff = DiffParser::parse_text_diff(old, new, PathBuf::from("test.txt")).unwrap();
        
        assert_eq!(diff.file_path, PathBuf::from("test.txt"));
        assert!(!diff.hunks.is_empty());
    }

    #[test]
    fn test_parse_unified_diff_without_git_header() {
        let diff_text = "\
--- a/foo.txt\n\
+++ b/foo.txt\n\
@@ -1,1 +1,1 @@\n\
-hello\n\
+world\n";

        let diffs = DiffParser::parse_unified_diff(diff_text).unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_path, PathBuf::from("foo.txt"));
        assert_eq!(diffs[0].hunks.len(), 1);
    }
}
