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
#[command(about = "A composable code review engine for automated diff analysis", long_about = None)]
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
    },
    Compare {
        #[arg(long)]
        old_file: PathBuf,
        
        #[arg(long)]
        new_file: PathBuf,
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
        Commands::Pr { number, repo, post_comments } => {
            pr_command(number, repo, post_comments, config, cli.output_format).await?;
        }
        Commands::Compare { old_file, new_file } => {
            compare_command(old_file, new_file, config, cli.output_format).await?;
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
    let mut prompt_config = core::prompt::PromptConfig::default();
    if let Some(custom_prompt) = &config.system_prompt {
        prompt_config.system_prompt = custom_prompt.clone();
    }
    let prompt_builder = core::PromptBuilder::new(prompt_config);
    let mut all_comments = Vec::new();
    
    for diff in diffs {
        let context_fetcher = core::ContextFetcher::new(PathBuf::from("."));
        let context_chunks = context_fetcher.fetch_context_for_file(
            &diff.file_path,
            &diff.hunks.iter()
                .map(|h| (h.new_start, h.new_start + h.new_lines))
                .collect::<Vec<_>>()
        ).await?;
        
        let (system_prompt, user_prompt) = prompt_builder.build_prompt(&diff, &context_chunks)?;
        
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
    let mut prompt_config = core::prompt::PromptConfig::default();
    if let Some(custom_prompt) = &config.system_prompt {
        prompt_config.system_prompt = custom_prompt.clone();
    }
    let prompt_builder = core::PromptBuilder::new(prompt_config);
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
        
        let (system_prompt, user_prompt) = prompt_builder.build_prompt(&diff, &context_chunks)?;
        
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
    output.push_str("# Code Review Results\n\n");
    
    for comment in comments {
        output.push_str(&format!(
            "## {}:{}\n\n**Severity:** {:?}\n**Category:** {:?}\n\n{}\n\n",
            comment.file_path.display(),
            comment.line_number,
            comment.severity,
            comment.category,
            comment.content
        ));
        
        if let Some(suggestion) = &comment.suggestion {
            output.push_str(&format!("**Suggestion:** {}\n\n", suggestion));
        }
    }
    
    output
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
