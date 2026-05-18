use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::{Comment, LLMContextChunk, UnifiedDiff};

use super::evidence::{
    diff_snippet_for_comment, line_is_removed_only_in_diff, source_context_for_line,
};
use super::support::supporting_context_for_comment;

pub(super) fn render_comment_section(
    index: usize,
    comment: &Comment,
    diff: Option<&UnifiedDiff>,
    source_files: &HashMap<PathBuf, String>,
    extra_context: &HashMap<PathBuf, Vec<LLMContextChunk>>,
) -> String {
    let mut section = format!(
        "### Finding {}\n<untrusted_review_finding index=\"{}\">\n- File: {}:{}\n- Issue: {}\n",
        index + 1,
        index + 1,
        comment.file_path.display(),
        comment.line_number,
        sanitize_untrusted_prompt_text(&comment.content),
    );

    if let Some(suggestion) = comment.suggestion.as_ref() {
        section.push_str(&format!(
            "- Suggestion: {}\n",
            sanitize_untrusted_prompt_text(suggestion)
        ));
    }
    section.push_str("</untrusted_review_finding>\n");

    if let Some(diff) = diff {
        let diff_snippet = diff_snippet_for_comment(diff, comment.line_number);
        append_code_block(&mut section, "- Diff evidence:\n", "diff", &diff_snippet);
    }

    let include_source_context = diff
        .map(|current_diff| !line_is_removed_only_in_diff(current_diff, comment.line_number))
        .unwrap_or(true);

    if include_source_context {
        if let Some(content) = source_files.get(&comment.file_path) {
            let file_context = source_context_for_line(content, comment.line_number, 6);
            append_code_block(&mut section, "- Nearby file context:\n", "", &file_context);
        }
    }

    let supporting_context = supporting_context_for_comment(comment, extra_context);
    if !supporting_context.is_empty() {
        section.push_str("- Cross-file attachment rule: if this changed line introduces a risky call or tainted input into the helper below, the finding can still be accurate and line-correct even when the vulnerable sink lives in the supporting-context file.\n");
        section.push_str("- Supporting context:\n");
        for chunk in supporting_context {
            section.push_str("```text\n");
            section.push_str(&format_context_chunk_for_verification(&chunk));
            section.push_str("\n```\n");
        }
    }

    section.push('\n');
    section
}

fn append_code_block(section: &mut String, label: &str, language: &str, content: &str) {
    if content.trim().is_empty() {
        return;
    }

    section.push_str(label);
    let fence = code_fence_for(content);
    section.push_str("<untrusted_code_evidence>\n");
    section.push_str(&format!("{fence}{language}\n"));
    section.push_str(content);
    section.push('\n');
    section.push_str(&fence);
    section.push_str("\n</untrusted_code_evidence>\n");
}

fn format_context_chunk_for_verification(chunk: &LLMContextChunk) -> String {
    let mut header = format!(
        "{:?} - {}{}",
        chunk.context_type,
        chunk.file_path.display(),
        chunk
            .line_range
            .map(|(start, end)| format!(":{start}-{end}"))
            .unwrap_or_default()
    );

    if let Some(provenance) = chunk.provenance.as_ref() {
        header.push_str(" | ");
        header.push_str(&provenance.to_string());
    }

    format!("{}\n{}", header, chunk.content)
}

fn sanitize_untrusted_prompt_text(value: &str) -> String {
    value
        .replace(
            "</untrusted_review_finding>",
            "<\\/untrusted_review_finding>",
        )
        .replace(
            "<untrusted_review_finding",
            "<untrusted_review_finding_text",
        )
}

fn code_fence_for(content: &str) -> String {
    let mut longest_run = 0usize;
    let mut current_run = 0usize;
    for ch in content.chars() {
        if ch == '`' {
            current_run += 1;
            longest_run = longest_run.max(current_run);
        } else {
            current_run = 0;
        }
    }
    let longest_run = longest_run.max(3);
    "`".repeat(longest_run + 1)
}
