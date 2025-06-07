mod core;
mod adapters;
mod plugins;
mod config;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "diffscope")]
#[command(about = "A composable code review engine with smart analysis and professional reporting", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    
    #[arg(long, global = true, default_value = "gpt-4o")]
    model: String,
    
    #[arg(long, global = true)]
    prompt: Option<String>,
    
    #[arg(long, global = true)]
    temperature: Option<f32>,
    
    #[arg(long, global = true)]
    max_tokens: Option<usize>,
    
    #[arg(long, global = true, default_value = "json")]
    output_format: OutputFormat,
    
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    Review {
        #[arg(long)]
        diff: Option<PathBuf>,
        
        #[arg(long)]
        patch: bool,
        
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    Check {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    Git {
        #[command(subcommand)]
        command: GitCommands,
    },
    Pr {
        #[arg(long)]
        number: Option<u32>,
        
        #[arg(long)]
        repo: Option<String>,
        
        #[arg(long)]
        post_comments: bool,
        
        #[arg(long)]
        summary: bool,
    },
    Compare {
        #[arg(long)]
        old_file: PathBuf,
        
        #[arg(long)]
        new_file: PathBuf,
    },
    #[command(about = "Enhanced code review with confidence scoring and executive summaries")]
    SmartReview {
        #[arg(long, help = "Path to diff file (reads from stdin if not provided)")]
        diff: Option<PathBuf>,
        
        #[arg(short, long, help = "Output file path (prints to stdout if not provided)")]
        output: Option<PathBuf>,
    },
    #[command(about = "Generate changelog and release notes from git history")]
    Changelog {
        #[arg(long, help = "Starting tag/commit (defaults to most recent tag)")]
        from: Option<String>,
        
        #[arg(long, help = "Ending ref (defaults to HEAD)")]
        to: Option<String>,
        
        #[arg(long, help = "Generate release notes for a specific version")]
        release: Option<String>,
        
        #[arg(short, long, help = "Output file path (prints to stdout if not provided)")]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum GitCommands {
    Uncommitted,
    Staged,
    Branch {
        #[arg(default_value = "main")]
        base: String,
    },
    Suggest,
    PrTitle,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum OutputFormat {
    Json,
    Patch,
    Markdown,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };
    
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();
    
    // Load configuration from file and merge with CLI options
    let mut config = config::Config::load().unwrap_or_default();
    config.merge_with_cli(Some(cli.model.clone()), cli.prompt.clone());
    
    // Override with CLI temperature and max_tokens if provided
    if let Some(temp) = cli.temperature {
        config.temperature = temp;
    }
    if let Some(tokens) = cli.max_tokens {
        config.max_tokens = tokens;
    }
    
    match cli.command {
        Commands::Review { diff, patch, output } => {
            review_command(config, diff, patch, output, cli.output_format).await?;
        }
        Commands::Check { path } => {
            check_command(path, config).await?;
        }
        Commands::Git { command } => {
            git_command(command, config, cli.output_format).await?;
        }
        Commands::Pr { number, repo, post_comments, summary } => {
            pr_command(number, repo, post_comments, summary, config, cli.output_format).await?;
        }
        Commands::Compare { old_file, new_file } => {
            compare_command(old_file, new_file, config, cli.output_format).await?;
        }
        Commands::SmartReview { diff, output } => {
            smart_review_command(config, diff, output).await?;
        }
        Commands::Changelog { from, to, release, output } => {
            changelog_command(from, to, release, output).await?;
        }
    }
    
    Ok(())
}

async fn review_command(
    config: config::Config,
    diff_path: Option<PathBuf>,
    _patch: bool,
    output_path: Option<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    info!("Starting diff review with model: {}", config.model);
    
    let diff_content = if let Some(path) = diff_path {
        tokio::fs::read_to_string(path).await?
    } else {
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        buffer
    };
    
    let diffs = core::DiffParser::parse_unified_diff(&diff_content)?;
    info!("Parsed {} file diffs", diffs.len());
    
    let model_config = adapters::llm::ModelConfig {
        model_name: config.model.clone(),
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
    };
    
    let adapter = adapters::llm::create_adapter(&model_config)?;
    let base_prompt_config = core::prompt::PromptConfig::default();
    let mut all_comments = Vec::new();
    
    for diff in diffs {
        // Check if file should be excluded
        if config.should_exclude(&diff.file_path) {
            info!("Skipping excluded file: {}", diff.file_path.display());
            continue;
        }
        
        let context_fetcher = core::ContextFetcher::new(PathBuf::from("."));
        let mut context_chunks = context_fetcher.fetch_context_for_file(
            &diff.file_path,
            &diff.hunks.iter()
                .map(|h| (h.new_start, h.new_start + h.new_lines))
                .collect::<Vec<_>>()
        ).await?;
        
        // Get path-specific configuration
        let path_config = config.get_path_config(&diff.file_path);
        
        // Apply path-specific system prompt if available
        let mut local_prompt_config = base_prompt_config.clone();
        if let Some(custom_prompt) = &config.system_prompt {
            local_prompt_config.system_prompt = custom_prompt.clone();
        }
        if let Some(pc) = path_config {
            if let Some(ref prompt) = pc.system_prompt {
                local_prompt_config.system_prompt = prompt.clone();
            }
            
            // Add focus areas to context
            if !pc.focus.is_empty() {
                let focus_chunk = core::LLMContextChunk {
                    content: format!("Focus areas for this file: {}", pc.focus.join(", ")),
                    context_type: core::ContextType::Documentation,
                    file_path: diff.file_path.clone(),
                    line_range: None,
                };
                context_chunks.push(focus_chunk);
            }
        }
        
        let local_prompt_builder = core::PromptBuilder::new(local_prompt_config);
        let (system_prompt, user_prompt) = local_prompt_builder.build_prompt(&diff, &context_chunks)?;
        
        let request = adapters::llm::LLMRequest {
            system_prompt,
            user_prompt,
            temperature: None,
            max_tokens: None,
        };
        
        let response = adapter.complete(request).await?;
        
        if let Ok(raw_comments) = parse_llm_response(&response.content, &diff.file_path) {
            let mut comments = core::CommentSynthesizer::synthesize(raw_comments)?;
            
            // Apply severity overrides if configured
            if let Some(pc) = path_config {
                for comment in &mut comments {
                    for (category, severity) in &pc.severity_overrides {
                        if format!("{:?}", comment.category).to_lowercase() == category.to_lowercase() {
                            comment.severity = match severity.to_lowercase().as_str() {
                                "error" => core::comment::Severity::Error,
                                "warning" => core::comment::Severity::Warning,
                                "info" => core::comment::Severity::Info,
                                "suggestion" => core::comment::Severity::Suggestion,
                                _ => comment.severity.clone(),
                            };
                        }
                    }
                }
            }
            
            all_comments.extend(comments);
        }
    }
    
    output_comments(&all_comments, output_path, format).await?;
    
    Ok(())
}

async fn check_command(path: PathBuf, config: config::Config) -> Result<()> {
    info!("Checking repository at: {}", path.display());
    info!("Using model: {}", config.model);
    
    println!("Repository check not yet implemented");
    
    Ok(())
}

async fn git_command(command: GitCommands, config: config::Config, format: OutputFormat) -> Result<()> {
    let git = core::GitIntegration::new(".")?;
    
    let diff_content = match command {
        GitCommands::Uncommitted => {
            info!("Analyzing uncommitted changes");
            git.get_uncommitted_diff()?
        }
        GitCommands::Staged => {
            info!("Analyzing staged changes");
            git.get_staged_diff()?
        }
        GitCommands::Branch { base } => {
            info!("Analyzing changes from branch: {}", base);
            git.get_branch_diff(&base)?
        }
        GitCommands::Suggest => {
            return suggest_commit_message(config).await;
        }
        GitCommands::PrTitle => {
            return suggest_pr_title(config).await;
        }
    };
    
    if diff_content.is_empty() {
        println!("No changes found");
        return Ok(());
    }
    
    review_diff_content(&diff_content, config, format).await
}

async fn pr_command(
    number: Option<u32>,
    _repo: Option<String>,
    post_comments: bool,
    summary: bool,
    config: config::Config,
    format: OutputFormat,
) -> Result<()> {
    use std::process::Command;
    
    let pr_number = if let Some(num) = number {
        num.to_string()
    } else {
        // Get current PR number
        let output = Command::new("gh")
            .args(&["pr", "view", "--json", "number", "-q", ".number"])
            .output()?;
        String::from_utf8(output.stdout)?.trim().to_string()
    };
    
    info!("Reviewing PR #{}", pr_number);
    
    // Get additional git context
    let git = core::GitIntegration::new(".")?;
    if let Ok(branch) = git.get_current_branch() {
        info!("Current branch: {}", branch);
    }
    if let Ok(Some(remote)) = git.get_remote_url() {
        info!("Remote URL: {}", remote);
    }
    
    // Get PR diff
    let diff_output = Command::new("gh")
        .args(&["pr", "diff", &pr_number])
        .output()?;
    
    let diff_content = String::from_utf8(diff_output.stdout)?;
    
    if diff_content.is_empty() {
        println!("No changes in PR");
        return Ok(());
    }
    
    // Generate PR summary if requested
    if summary {
        let diffs = core::DiffParser::parse_unified_diff(&diff_content)?;
        let git = core::GitIntegration::new(".")?;
        
        let model_config = adapters::llm::ModelConfig {
            model_name: config.model.clone(),
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
        };
        
        let adapter = adapters::llm::create_adapter(&model_config)?;
        let pr_summary = core::PRSummaryGenerator::generate_summary(&diffs, &git, &adapter).await?;
        
        println!("{}", pr_summary.to_markdown());
        return Ok(());
    }
    
    let comments = review_diff_content_raw(&diff_content, config.clone()).await?;
    
    if post_comments && !comments.is_empty() {
        info!("Posting {} comments to PR", comments.len());
        
        for comment in &comments {
            let body = format!("**{}**: {}", 
                format!("{:?}", comment.severity), 
                comment.content
            );
            
            Command::new("gh")
                .args(&[
                    "pr", "comment", &pr_number,
                    "--body", &body
                ])
                .output()?;
        }
        
        println!("Posted {} comments to PR #{}", comments.len(), pr_number);
    } else {
        output_comments(&comments, None, format).await?;
    }
    
    Ok(())
}

async fn suggest_commit_message(config: config::Config) -> Result<()> {
    let git = core::GitIntegration::new(".")?;
    let diff_content = git.get_staged_diff()?;
    
    if diff_content.is_empty() {
        println!("No staged changes found. Stage your changes with 'git add' first.");
        return Ok(());
    }
    
    let model_config = adapters::llm::ModelConfig {
        model_name: config.model.clone(),
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
    };
    
    let adapter = adapters::llm::create_adapter(&model_config)?;
    
    let (system_prompt, user_prompt) = core::CommitPromptBuilder::build_commit_prompt(&diff_content);
    
    let request = adapters::llm::LLMRequest {
        system_prompt,
        user_prompt,
        temperature: Some(0.3),
        max_tokens: Some(500),
    };
    
    let response = adapter.complete(request).await?;
    let commit_message = core::CommitPromptBuilder::extract_commit_message(&response.content);
    
    println!("\nSuggested commit message:");
    println!("{}", commit_message);
    
    if commit_message.len() > 72 {
        println!("\n⚠️  Warning: Commit message exceeds 72 characters ({})", commit_message.len());
    }
    
    Ok(())
}

async fn suggest_pr_title(config: config::Config) -> Result<()> {
    let git = core::GitIntegration::new(".")?;
    let diff_content = git.get_branch_diff("main")?;
    
    if diff_content.is_empty() {
        println!("No changes found compared to main branch.");
        return Ok(());
    }
    
    let model_config = adapters::llm::ModelConfig {
        model_name: config.model.clone(),
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
    };
    
    let adapter = adapters::llm::create_adapter(&model_config)?;
    
    let (system_prompt, user_prompt) = core::CommitPromptBuilder::build_pr_title_prompt(&diff_content);
    
    let request = adapters::llm::LLMRequest {
        system_prompt,
        user_prompt,
        temperature: Some(0.3),
        max_tokens: Some(200),
    };
    
    let response = adapter.complete(request).await?;
    
    // Extract title from response
    let title = if let Some(start) = response.content.find("<title>") {
        if let Some(end) = response.content.find("</title>") {
            response.content[start + 7..end].trim().to_string()
        } else {
            response.content.trim().to_string()
        }
    } else {
        // Fallback: take the first non-empty line
        response.content
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    
    println!("\nSuggested PR title:");
    println!("{}", title);
    
    if title.len() > 65 {
        println!("\n⚠️  Warning: PR title exceeds 65 characters ({})", title.len());
    }
    
    Ok(())
}

async fn compare_command(
    old_file: PathBuf,
    new_file: PathBuf,
    config: config::Config,
    format: OutputFormat,
) -> Result<()> {
    info!("Comparing files: {} vs {}", old_file.display(), new_file.display());
    
    let old_content = tokio::fs::read_to_string(&old_file).await?;
    let new_content = tokio::fs::read_to_string(&new_file).await?;
    
    // Use the parse_text_diff function to create a UnifiedDiff
    let diff = core::DiffParser::parse_text_diff(&old_content, &new_content, new_file.clone())?;
    
    // Convert the diff to a string format for the review process
    let diff_string = format!(
        "--- {}\n+++ {}\n{}",
        old_file.display(),
        new_file.display(),
        format_diff_as_unified(&diff)
    );
    
    review_diff_content(&diff_string, config, format).await
}

fn format_diff_as_unified(diff: &core::UnifiedDiff) -> String {
    let mut output = String::new();
    
    for hunk in &diff.hunks {
        output.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_lines,
            hunk.new_start, hunk.new_lines
        ));
        
        for line in &hunk.changes {
            let prefix = match line.change_type {
                core::diff_parser::ChangeType::Added => "+",
                core::diff_parser::ChangeType::Removed => "-",
                core::diff_parser::ChangeType::Context => " ",
            };
            output.push_str(&format!("{}{}\n", prefix, line.content));
        }
    }
    
    output
}

async fn review_diff_content(
    diff_content: &str,
    config: config::Config,
    format: OutputFormat,
) -> Result<()> {
    let comments = review_diff_content_raw(diff_content, config).await?;
    output_comments(&comments, None, format).await
}

async fn review_diff_content_raw(
    diff_content: &str,
    config: config::Config,
) -> Result<Vec<core::Comment>> {
    let diffs = core::DiffParser::parse_unified_diff(diff_content)?;
    info!("Parsed {} file diffs", diffs.len());
    
    // Initialize plugin manager and load builtin plugins
    let mut plugin_manager = plugins::plugin::PluginManager::new();
    plugin_manager.load_builtin_plugins().await?;
    
    let model_config = adapters::llm::ModelConfig {
        model_name: config.model.clone(),
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
    };
    
    let adapter = adapters::llm::create_adapter(&model_config)?;
    let base_prompt_config = core::prompt::PromptConfig::default();
    let mut all_comments = Vec::new();
    
    for diff in diffs {
        let context_fetcher = core::ContextFetcher::new(PathBuf::from("."));
        let mut context_chunks = context_fetcher.fetch_context_for_file(
            &diff.file_path,
            &diff.hunks.iter()
                .map(|h| (h.new_start, h.new_start + h.new_lines))
                .collect::<Vec<_>>()
        ).await?;
        
        // Run pre-analyzers to get additional context
        let analyzer_chunks = plugin_manager.run_pre_analyzers(&diff, ".").await?;
        context_chunks.extend(analyzer_chunks);
        
        // Extract symbols from diff and fetch their definitions
        let symbols = extract_symbols_from_diff(&diff);
        if !symbols.is_empty() {
            let definition_chunks = context_fetcher.fetch_related_definitions(&diff.file_path, &symbols).await?;
            context_chunks.extend(definition_chunks);
        }
        
        // Create prompt builder with config
        let mut local_prompt_config = base_prompt_config.clone();
        if let Some(custom_prompt) = &config.system_prompt {
            local_prompt_config.system_prompt = custom_prompt.clone();
        }
        let local_prompt_builder = core::PromptBuilder::new(local_prompt_config);
        let (system_prompt, user_prompt) = local_prompt_builder.build_prompt(&diff, &context_chunks)?;
        
        let request = adapters::llm::LLMRequest {
            system_prompt,
            user_prompt,
            temperature: None,
            max_tokens: None,
        };
        
        let response = adapter.complete(request).await?;
        
        if let Ok(raw_comments) = parse_llm_response(&response.content, &diff.file_path) {
            let comments = core::CommentSynthesizer::synthesize(raw_comments)?;
            all_comments.extend(comments);
        }
    }
    
    // Run post-processors to filter and refine comments
    let processed_comments = plugin_manager.run_post_processors(all_comments, ".").await?;
    
    Ok(processed_comments)
}

fn parse_llm_response(content: &str, file_path: &PathBuf) -> Result<Vec<core::comment::RawComment>> {
    let mut comments = Vec::new();
    let line_pattern = regex::Regex::new(r"(?i)line\s+(\d+):\s*(.+)")?;
    
    for line in content.lines() {
        let trimmed = line.trim();
        
        // Skip empty lines and common non-issue lines
        if trimmed.is_empty() || 
           trimmed.starts_with("```") || 
           trimmed.starts_with('#') ||
           trimmed.starts_with('<') ||
           trimmed.contains("Here are") ||
           trimmed.contains("Here is") ||
           trimmed.contains("review of") {
            continue;
        }
        
        if let Some(caps) = line_pattern.captures(line) {
            let line_number: usize = caps.get(1).unwrap().as_str().parse()?;
            let comment_text = caps.get(2).unwrap().as_str().trim();
            
            // Extract suggestion if present
            let (content, suggestion) = if let Some(sugg_idx) = comment_text.rfind(". Consider ") {
                (
                    comment_text[..sugg_idx + 1].to_string(),
                    Some(comment_text[sugg_idx + 11..].trim_end_matches('.').to_string())
                )
            } else if let Some(sugg_idx) = comment_text.rfind(". Use ") {
                (
                    comment_text[..sugg_idx + 1].to_string(),
                    Some(comment_text[sugg_idx + 6..].trim_end_matches('.').to_string())
                )
            } else {
                (comment_text.to_string(), None)
            };
            
            comments.push(core::comment::RawComment {
                file_path: file_path.clone(),
                line_number,
                content,
                suggestion,
            });
        }
    }
    
    Ok(comments)
}

async fn output_comments(
    comments: &[core::Comment],
    output_path: Option<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    let output = match format {
        OutputFormat::Json => serde_json::to_string_pretty(comments)?,
        OutputFormat::Patch => format_as_patch(comments),
        OutputFormat::Markdown => format_as_markdown(comments),
    };
    
    if let Some(path) = output_path {
        tokio::fs::write(path, output).await?;
    } else {
        println!("{}", output);
    }
    
    Ok(())
}

fn format_as_patch(comments: &[core::Comment]) -> String {
    let mut output = String::new();
    for comment in comments {
        output.push_str(&format!(
            "# {}:{} - {:?}\n# {}\n",
            comment.file_path.display(),
            comment.line_number,
            comment.severity,
            comment.content
        ));
    }
    output
}

fn format_as_markdown(comments: &[core::Comment]) -> String {
    let mut output = String::new();
    
    // Generate summary
    let summary = core::CommentSynthesizer::generate_summary(comments);
    
    output.push_str("# Code Review Results\n\n");
    output.push_str(&format!("## Summary\n\n"));
    output.push_str(&format!("📊 **Overall Score:** {:.1}/10\n", summary.overall_score));
    output.push_str(&format!("📝 **Total Issues:** {}\n", summary.total_comments));
    output.push_str(&format!("🚨 **Critical Issues:** {}\n", summary.critical_issues));
    output.push_str(&format!("📁 **Files Reviewed:** {}\n\n", summary.files_reviewed));
    
    // Severity breakdown
    output.push_str("### Issues by Severity\n\n");
    for (severity, count) in &summary.by_severity {
        let emoji = match severity.as_str() {
            "Error" => "🔴",
            "Warning" => "🟡", 
            "Info" => "🔵",
            "Suggestion" => "💡",
            _ => "⚪",
        };
        output.push_str(&format!("{} **{}:** {}\n", emoji, severity, count));
    }
    output.push_str("\n");
    
    // Category breakdown  
    output.push_str("### Issues by Category\n\n");
    for (category, count) in &summary.by_category {
        let emoji = match category.as_str() {
            "Security" => "🔒",
            "Performance" => "⚡",
            "Bug" => "🐛",
            "Style" => "🎨",
            "Documentation" => "📚",
            "Testing" => "🧪",
            "Maintainability" => "🔧",
            "Architecture" => "🏗️",
            _ => "💭",
        };
        output.push_str(&format!("{} **{}:** {}\n", emoji, category, count));
    }
    output.push_str("\n");
    
    // Recommendations
    if !summary.recommendations.is_empty() {
        output.push_str("### Recommendations\n\n");
        for rec in &summary.recommendations {
            output.push_str(&format!("- {}\n", rec));
        }
        output.push_str("\n");
    }
    
    output.push_str("---\n\n## Detailed Issues\n\n");
    
    // Group comments by file
    let mut comments_by_file = std::collections::HashMap::new();
    for comment in comments {
        comments_by_file.entry(&comment.file_path)
            .or_insert_with(Vec::new)
            .push(comment);
    }
    
    for (file_path, file_comments) in comments_by_file {
        output.push_str(&format!("### {}\n\n", file_path.display()));
        
        for comment in file_comments {
            let severity_emoji = match comment.severity {
                core::comment::Severity::Error => "🔴",
                core::comment::Severity::Warning => "🟡",
                core::comment::Severity::Info => "🔵", 
                core::comment::Severity::Suggestion => "💡",
            };
            
            let effort_badge = match comment.fix_effort {
                core::comment::FixEffort::Low => "🟢 Quick Fix",
                core::comment::FixEffort::Medium => "🟡 Moderate",
                core::comment::FixEffort::High => "🔴 Complex",
            };
            
            output.push_str(&format!(
                "#### Line {} {} {:?}\n\n",
                comment.line_number,
                severity_emoji,
                comment.category
            ));
            
            output.push_str(&format!("**Confidence:** {:.0}%\n", comment.confidence * 100.0));
            output.push_str(&format!("**Fix Effort:** {}\n\n", effort_badge));
            
            output.push_str(&format!("{}\n\n", comment.content));
            
            if let Some(suggestion) = &comment.suggestion {
                output.push_str(&format!("💡 **Suggestion:** {}\n\n", suggestion));
            }
            
            if let Some(code_suggestion) = &comment.code_suggestion {
                output.push_str("**Code Suggestion:**\n");
                output.push_str(&format!("```diff\n{}\n```\n\n", code_suggestion.diff));
                output.push_str(&format!("_{}_ \n\n", code_suggestion.explanation));
            }
            
            if !comment.tags.is_empty() {
                output.push_str("**Tags:** ");
                for (i, tag) in comment.tags.iter().enumerate() {
                    if i > 0 { output.push_str(", "); }
                    output.push_str(&format!("`{}`", tag));
                }
                output.push_str("\n\n");
            }
            
            output.push_str("---\n\n");
        }
    }
    
    output
}

async fn smart_review_command(
    config: config::Config,
    diff_path: Option<PathBuf>,
    output_path: Option<PathBuf>,
) -> Result<()> {
    info!("Starting smart review analysis with model: {}", config.model);
    
    let diff_content = if let Some(path) = diff_path {
        tokio::fs::read_to_string(path).await?
    } else {
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        buffer
    };
    
    let diffs = core::DiffParser::parse_unified_diff(&diff_content)?;
    info!("Parsed {} file diffs", diffs.len());
    
    let model_config = adapters::llm::ModelConfig {
        model_name: config.model.clone(),
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
    };
    
    let adapter = adapters::llm::create_adapter(&model_config)?;
    let mut all_comments = Vec::new();
    
    for diff in diffs {
        // Check if file should be excluded
        if config.should_exclude(&diff.file_path) {
            info!("Skipping excluded file: {}", diff.file_path.display());
            continue;
        }
        
        let context_fetcher = core::ContextFetcher::new(PathBuf::from("."));
        let mut context_chunks = context_fetcher.fetch_context_for_file(
            &diff.file_path,
            &diff.hunks.iter()
                .map(|h| (h.new_start, h.new_start + h.new_lines))
                .collect::<Vec<_>>()
        ).await?;
        
        // Get path-specific configuration
        let path_config = config.get_path_config(&diff.file_path);
        
        // Add focus areas to context if configured
        if let Some(pc) = path_config {
            if !pc.focus.is_empty() {
                let focus_chunk = core::LLMContextChunk {
                    content: format!("Focus areas for this file: {}", pc.focus.join(", ")),
                    context_type: core::ContextType::Documentation,
                    file_path: diff.file_path.clone(),
                    line_range: None,
                };
                context_chunks.push(focus_chunk);
            }
        }
        
        // Extract symbols and get definitions
        let symbols = extract_symbols_from_diff(&diff);
        if !symbols.is_empty() {
            let definition_chunks = context_fetcher.fetch_related_definitions(&diff.file_path, &symbols).await?;
            context_chunks.extend(definition_chunks);
        }
        
        let (system_prompt, user_prompt) = core::SmartReviewPromptBuilder::build_enhanced_review_prompt(&diff, &context_chunks)?;
        
        let request = adapters::llm::LLMRequest {
            system_prompt,
            user_prompt,
            temperature: Some(0.2), // Lower temperature for more consistent analysis
            max_tokens: Some(4000),
        };
        
        let response = adapter.complete(request).await?;
        
        if let Ok(raw_comments) = parse_smart_review_response(&response.content, &diff.file_path) {
            let mut comments = core::CommentSynthesizer::synthesize(raw_comments)?;
            
            // Apply severity overrides if configured
            if let Some(pc) = path_config {
                for comment in &mut comments {
                    for (category, severity) in &pc.severity_overrides {
                        if format!("{:?}", comment.category).to_lowercase() == category.to_lowercase() {
                            comment.severity = match severity.to_lowercase().as_str() {
                                "error" => core::comment::Severity::Error,
                                "warning" => core::comment::Severity::Warning,
                                "info" => core::comment::Severity::Info,
                                "suggestion" => core::comment::Severity::Suggestion,
                                _ => comment.severity.clone(),
                            };
                        }
                    }
                }
            }
            
            all_comments.extend(comments);
        }
    }
    
    // Generate summary and output results
    let summary = core::CommentSynthesizer::generate_summary(&all_comments);
    let output = format_smart_review_output(&all_comments, &summary);
    
    if let Some(path) = output_path {
        tokio::fs::write(path, output).await?;
    } else {
        println!("{}", output);
    }
    
    Ok(())
}

fn parse_smart_review_response(content: &str, file_path: &PathBuf) -> Result<Vec<core::comment::RawComment>> {
    let mut comments = Vec::new();
    let mut current_comment: Option<core::comment::RawComment> = None;
    
    for line in content.lines() {
        let trimmed = line.trim();
        
        if trimmed.starts_with("ISSUE:") {
            // Save previous comment if exists
            if let Some(comment) = current_comment.take() {
                comments.push(comment);
            }
            
            // Start new comment
            let title = trimmed.strip_prefix("ISSUE:").unwrap_or("").trim();
            current_comment = Some(core::comment::RawComment {
                file_path: file_path.clone(),
                line_number: 1,
                content: title.to_string(),
                suggestion: None,
            });
        } else if trimmed.starts_with("LINE:") {
            if let Some(ref mut comment) = current_comment {
                if let Ok(line_num) = trimmed.strip_prefix("LINE:").unwrap_or("").trim().parse::<usize>() {
                    comment.line_number = line_num;
                }
            }
        } else if trimmed.starts_with("DESCRIPTION:") {
            // Start collecting description on next line
            continue;
        } else if trimmed.starts_with("SUGGESTION:") {
            // Start collecting suggestion on next line  
            continue;
        } else if !trimmed.is_empty() && 
                  !trimmed.starts_with("SEVERITY:") && 
                  !trimmed.starts_with("CATEGORY:") &&
                  !trimmed.starts_with("CONFIDENCE:") &&
                  !trimmed.starts_with("EFFORT:") &&
                  !trimmed.starts_with("TAGS:") {
            // This is content - add to current comment
            if let Some(ref mut comment) = current_comment {
                if !comment.content.is_empty() {
                    comment.content.push(' ');
                }
                comment.content.push_str(trimmed);
            }
        }
    }
    
    // Save last comment
    if let Some(comment) = current_comment {
        comments.push(comment);
    }
    
    Ok(comments)
}

fn format_smart_review_output(comments: &[core::Comment], summary: &core::comment::ReviewSummary) -> String {
    let mut output = String::new();
    
    output.push_str("# 🤖 Smart Review Analysis Results\n\n");
    
    // Executive Summary
    output.push_str("## 📊 Executive Summary\n\n");
    let score_emoji = if summary.overall_score >= 8.0 { "🟢" } else if summary.overall_score >= 6.0 { "🟡" } else { "🔴" };
    output.push_str(&format!("{} **Code Quality Score:** {:.1}/10\n", score_emoji, summary.overall_score));
    output.push_str(&format!("📝 **Total Issues Found:** {}\n", summary.total_comments));
    output.push_str(&format!("🚨 **Critical Issues:** {}\n", summary.critical_issues));
    output.push_str(&format!("📁 **Files Analyzed:** {}\n\n", summary.files_reviewed));
    
    // Quick Stats
    output.push_str("### 📈 Issue Breakdown\n\n");
    output.push_str("| Severity | Count | Category | Count |\n");
    output.push_str("|----------|-------|----------|-------|\n");
    
    let severities = ["Error", "Warning", "Info", "Suggestion"];
    let categories = ["Security", "Performance", "Bug", "Maintainability"];
    
    for (i, severity) in severities.iter().enumerate() {
        let sev_count = summary.by_severity.get(*severity).unwrap_or(&0);
        let cat = categories.get(i).unwrap_or(&"");
        let cat_count = summary.by_category.get(*cat).unwrap_or(&0);
        
        output.push_str(&format!("| {} | {} | {} | {} |\n", severity, sev_count, cat, cat_count));
    }
    output.push_str("\n");
    
    // Actionable Recommendations
    if !summary.recommendations.is_empty() {
        output.push_str("### 🎯 Priority Actions\n\n");
        for (i, rec) in summary.recommendations.iter().enumerate() {
            output.push_str(&format!("{}. {}\n", i + 1, rec));
        }
        output.push_str("\n");
    }
    
    if comments.is_empty() {
        output.push_str("✅ **No issues found!** Your code looks good.\n");
        return output;
    }
    
    output.push_str("---\n\n## 🔍 Detailed Analysis\n\n");
    
    // Group by severity for better organization
    let mut critical_issues = Vec::new();
    let mut high_issues = Vec::new();
    let mut medium_issues = Vec::new();
    let mut low_issues = Vec::new();
    
    for comment in comments {
        match comment.severity {
            core::comment::Severity::Error => critical_issues.push(comment),
            core::comment::Severity::Warning => high_issues.push(comment),
            core::comment::Severity::Info => medium_issues.push(comment),
            core::comment::Severity::Suggestion => low_issues.push(comment),
        }
    }
    
    // Output each severity group
    if !critical_issues.is_empty() {
        output.push_str("### 🔴 Critical Issues (Fix Immediately)\n\n");
        for comment in critical_issues {
            output.push_str(&format_detailed_comment(comment));
        }
    }
    
    if !high_issues.is_empty() {
        output.push_str("### 🟡 High Priority Issues\n\n");
        for comment in high_issues {
            output.push_str(&format_detailed_comment(comment));
        }
    }
    
    if !medium_issues.is_empty() {
        output.push_str("### 🔵 Medium Priority Issues\n\n");
        for comment in medium_issues {
            output.push_str(&format_detailed_comment(comment));
        }
    }
    
    if !low_issues.is_empty() {
        output.push_str("### 💡 Suggestions & Improvements\n\n");
        for comment in low_issues {
            output.push_str(&format_detailed_comment(comment));
        }
    }
    
    output
}

fn format_detailed_comment(comment: &core::Comment) -> String {
    let mut output = String::new();
    
    let category_emoji = match comment.category {
        core::comment::Category::Security => "🔒",
        core::comment::Category::Performance => "⚡",
        core::comment::Category::Bug => "🐛",
        core::comment::Category::Style => "🎨",
        core::comment::Category::Documentation => "📚",
        core::comment::Category::Testing => "🧪",
        core::comment::Category::Maintainability => "🔧",
        core::comment::Category::Architecture => "🏗️",
        _ => "💭",
    };
    
    let effort_badge = match comment.fix_effort {
        core::comment::FixEffort::Low => "🟢 Quick Fix",
        core::comment::FixEffort::Medium => "🟡 Moderate Effort", 
        core::comment::FixEffort::High => "🔴 Significant Effort",
    };
    
    output.push_str(&format!(
        "#### {} **{}:{}** - {} {:?}\n\n",
        category_emoji,
        comment.file_path.display(),
        comment.line_number,
        effort_badge,
        comment.category
    ));
    
    output.push_str(&format!("**Confidence:** {:.0}% | ", comment.confidence * 100.0));
    if !comment.tags.is_empty() {
        output.push_str("**Tags:** ");
        for (i, tag) in comment.tags.iter().enumerate() {
            if i > 0 { output.push_str(", "); }
            output.push_str(&format!("`{}`", tag));
        }
    }
    output.push_str("\n\n");
    
    output.push_str(&format!("{}\n\n", comment.content));
    
    if let Some(suggestion) = &comment.suggestion {
        output.push_str(&format!("**💡 Recommended Fix:**\n{}\n\n", suggestion));
    }
    
    if let Some(code_suggestion) = &comment.code_suggestion {
        output.push_str("**🔧 Code Example:**\n");
        output.push_str(&format!("```diff\n{}\n```\n", code_suggestion.diff));
        output.push_str(&format!("_{}_\n\n", code_suggestion.explanation));
    }
    
    output.push_str("---\n\n");
    output
}

async fn changelog_command(
    from: Option<String>,
    to: Option<String>,
    release: Option<String>,
    output_path: Option<PathBuf>,
) -> Result<()> {
    info!("Generating changelog/release notes");
    
    let generator = core::ChangelogGenerator::new(".")?;
    
    let output = if let Some(version) = release {
        // Generate release notes
        info!("Generating release notes for version {}", version);
        generator.generate_release_notes(&version, from.as_deref())?
    } else {
        // Generate changelog
        let to_ref = to.as_deref().unwrap_or("HEAD");
        info!("Generating changelog from {:?} to {}", from, to_ref);
        generator.generate_changelog(from.as_deref(), to_ref)?
    };
    
    if let Some(path) = output_path {
        tokio::fs::write(path, output).await?;
        info!("Changelog written to file");
    } else {
        println!("{}", output);
    }
    
    Ok(())
}

fn extract_symbols_from_diff(diff: &core::UnifiedDiff) -> Vec<String> {
    let mut symbols = Vec::new();
    let symbol_regex = regex::Regex::new(r"\b([A-Z][a-zA-Z0-9_]*|[a-z][a-zA-Z0-9_]*)\s*\(").unwrap();
    
    for hunk in &diff.hunks {
        for line in &hunk.changes {
            if matches!(line.change_type, core::diff_parser::ChangeType::Added | core::diff_parser::ChangeType::Removed) {
                // Extract function calls and references
                for capture in symbol_regex.captures_iter(&line.content) {
                    if let Some(symbol) = capture.get(1) {
                        let symbol_str = symbol.as_str().to_string();
                        if symbol_str.len() > 2 && !symbols.contains(&symbol_str) {
                            symbols.push(symbol_str);
                        }
                    }
                }
                
                // Also look for class/struct references
                let class_regex = regex::Regex::new(r"\b(class|struct|interface|enum)\s+([A-Z][a-zA-Z0-9_]*)").unwrap();
                for capture in class_regex.captures_iter(&line.content) {
                    if let Some(class_name) = capture.get(2) {
                        let class_str = class_name.as_str().to_string();
                        if !symbols.contains(&class_str) {
                            symbols.push(class_str);
                        }
                    }
                }
            }
        }
    }
    
    symbols
}
