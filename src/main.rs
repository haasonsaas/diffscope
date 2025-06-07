mod core;
mod adapters;
mod plugins;

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
    prompt: Option<PathBuf>,
    
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
    
    match cli.command {
        Commands::Review { diff, patch, output } => {
            review_command(cli.model, diff, patch, output, cli.output_format).await?;
        }
        Commands::Check { path } => {
            check_command(path, cli.model).await?;
        }
        Commands::Git { command } => {
            git_command(command, cli.model, cli.output_format).await?;
        }
        Commands::Pr { number, repo, post_comments } => {
            pr_command(number, repo, post_comments, cli.model, cli.output_format).await?;
        }
    }
    
    Ok(())
}

async fn review_command(
    model: String,
    diff_path: Option<PathBuf>,
    _patch: bool,
    output_path: Option<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    info!("Starting diff review with model: {}", model);
    
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
        model_name: model,
        ..Default::default()
    };
    
    let adapter = adapters::llm::create_adapter(&model_config)?;
    let prompt_builder = core::PromptBuilder::new(core::prompt::PromptConfig::default());
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

async fn check_command(path: PathBuf, model: String) -> Result<()> {
    info!("Checking repository at: {}", path.display());
    info!("Using model: {}", model);
    
    println!("Repository check not yet implemented");
    
    Ok(())
}

async fn git_command(command: GitCommands, model: String, format: OutputFormat) -> Result<()> {
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
            return suggest_commit_message(model).await;
        }
        GitCommands::PrTitle => {
            return suggest_pr_title(model).await;
        }
    };
    
    if diff_content.is_empty() {
        println!("No changes found");
        return Ok(());
    }
    
    review_diff_content(&diff_content, model, format).await
}

async fn pr_command(
    number: Option<u32>,
    _repo: Option<String>,
    post_comments: bool,
    model: String,
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
    
    // Get PR diff
    let diff_output = Command::new("gh")
        .args(&["pr", "diff", &pr_number])
        .output()?;
    
    let diff_content = String::from_utf8(diff_output.stdout)?;
    
    if diff_content.is_empty() {
        println!("No changes in PR");
        return Ok(());
    }
    
    let comments = review_diff_content_raw(&diff_content, model.clone()).await?;
    
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

async fn suggest_commit_message(model: String) -> Result<()> {
    let git = core::GitIntegration::new(".")?;
    let diff_content = git.get_staged_diff()?;
    
    if diff_content.is_empty() {
        println!("No staged changes found. Stage your changes with 'git add' first.");
        return Ok(());
    }
    
    let model_config = adapters::llm::ModelConfig {
        model_name: model,
        ..Default::default()
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

async fn suggest_pr_title(model: String) -> Result<()> {
    let git = core::GitIntegration::new(".")?;
    let diff_content = git.get_branch_diff("main")?;
    
    if diff_content.is_empty() {
        println!("No changes found compared to main branch.");
        return Ok(());
    }
    
    let model_config = adapters::llm::ModelConfig {
        model_name: model,
        ..Default::default()
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

async fn review_diff_content(
    diff_content: &str,
    model: String,
    format: OutputFormat,
) -> Result<()> {
    let comments = review_diff_content_raw(diff_content, model).await?;
    output_comments(&comments, None, format).await
}

async fn review_diff_content_raw(
    diff_content: &str,
    model: String,
) -> Result<Vec<core::Comment>> {
    let diffs = core::DiffParser::parse_unified_diff(diff_content)?;
    info!("Parsed {} file diffs", diffs.len());
    
    // Initialize plugin manager and load builtin plugins
    let mut plugin_manager = plugins::plugin::PluginManager::new();
    plugin_manager.load_builtin_plugins().await?;
    
    let model_config = adapters::llm::ModelConfig {
        model_name: model,
        ..Default::default()
    };
    
    let adapter = adapters::llm::create_adapter(&model_config)?;
    let prompt_builder = core::PromptBuilder::new(core::prompt::PromptConfig::default());
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
