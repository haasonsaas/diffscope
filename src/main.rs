mod adapters;
mod config;
mod core;
mod plugins;

use anyhow::Result;
use clap::{Parser, Subcommand};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use tracing::{info, warn};
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

    #[arg(
        long,
        global = true,
        value_parser = clap::value_parser!(bool),
        help = "Use OpenAI Responses API (true/false)"
    )]
    openai_responses: Option<bool>,

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

        #[arg(
            short,
            long,
            help = "Output file path (prints to stdout if not provided)"
        )]
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

        #[arg(
            short,
            long,
            help = "Output file path (prints to stdout if not provided)"
        )]
        output: Option<PathBuf>,
    },
    Feedback {
        #[arg(
            long,
            value_name = "FILE",
            help = "Mark review JSON comments as accepted"
        )]
        accept: Option<PathBuf>,

        #[arg(
            long,
            value_name = "FILE",
            help = "Mark review JSON comments as rejected"
        )]
        reject: Option<PathBuf>,

        #[arg(long, help = "Override feedback file path")]
        feedback_path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum GitCommands {
    Uncommitted,
    Staged,
    Branch {
        #[arg(help = "Base branch/ref (defaults to repo default)")]
        base: Option<String>,
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

    tracing_subscriber::fmt().with_env_filter(filter).init();

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
    if let Some(flag) = cli.openai_responses {
        config.openai_use_responses = Some(flag);
    }
    config.normalize();

    match cli.command {
        Commands::Review {
            diff,
            patch,
            output,
        } => {
            review_command(config, diff, patch, output, cli.output_format).await?;
        }
        Commands::Check { path } => {
            check_command(path, config, cli.output_format).await?;
        }
        Commands::Git { command } => {
            git_command(command, config, cli.output_format).await?;
        }
        Commands::Pr {
            number,
            repo,
            post_comments,
            summary,
        } => {
            pr_command(
                number,
                repo,
                post_comments,
                summary,
                config,
                cli.output_format,
            )
            .await?;
        }
        Commands::Compare { old_file, new_file } => {
            compare_command(old_file, new_file, config, cli.output_format).await?;
        }
        Commands::SmartReview { diff, output } => {
            smart_review_command(config, diff, output).await?;
        }
        Commands::Changelog {
            from,
            to,
            release,
            output,
        } => {
            changelog_command(from, to, release, output).await?;
        }
        Commands::Feedback {
            accept,
            reject,
            feedback_path,
        } => {
            feedback_command(config, accept, reject, feedback_path).await?;
        }
    }

    Ok(())
}

async fn review_command(
    config: config::Config,
    diff_path: Option<PathBuf>,
    patch: bool,
    output_path: Option<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    info!("Starting diff review with model: {}", config.model);

    let repo_root = core::GitIntegration::new(".")
        .ok()
        .and_then(|git| git.workdir())
        .unwrap_or_else(|| PathBuf::from("."));
    let repo_path_str = repo_root.to_string_lossy().to_string();
    let context_fetcher = core::ContextFetcher::new(repo_root.clone());

    let mut plugin_manager = plugins::plugin::PluginManager::new();
    plugin_manager.load_builtin_plugins(&config.plugins).await?;
    let feedback = load_feedback_store(&config);

    let diff_content = if let Some(path) = diff_path {
        tokio::fs::read_to_string(path).await?
    } else if std::io::stdin().is_terminal() {
        if let Ok(git) = core::GitIntegration::new(".") {
            let diff = git.get_uncommitted_diff()?;
            if diff.is_empty() {
                println!("No changes found");
                return Ok(());
            }
            diff
        } else {
            println!("No diff provided and not in a git repository.");
            return Ok(());
        }
    } else {
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        buffer
    };

    let diffs = core::DiffParser::parse_unified_diff(&diff_content)?;
    info!("Parsed {} file diffs", diffs.len());
    let symbol_index = build_symbol_index(&config, &repo_root);
    let model_config = adapters::llm::ModelConfig {
        model_name: config.model.clone(),
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
        openai_use_responses: config.openai_use_responses,
    };

    let adapter = adapters::llm::create_adapter(&model_config)?;
    let mut base_prompt_config = core::prompt::PromptConfig::default();
    base_prompt_config.max_context_chars = config.max_context_chars;
    base_prompt_config.max_diff_chars = config.max_diff_chars;
    let mut all_comments = Vec::new();

    for diff in &diffs {
        // Check if file should be excluded
        if config.should_exclude(&diff.file_path) {
            info!("Skipping excluded file: {}", diff.file_path.display());
            continue;
        }
        if diff.is_deleted {
            info!("Skipping deleted file: {}", diff.file_path.display());
            continue;
        }
        if diff.is_binary || diff.hunks.is_empty() {
            info!("Skipping non-text diff: {}", diff.file_path.display());
            continue;
        }

        let mut context_chunks = context_fetcher
            .fetch_context_for_file(
                &diff.file_path,
                &diff
                    .hunks
                    .iter()
                    .map(|h| (h.new_start, h.new_start + h.new_lines.saturating_sub(1)))
                    .collect::<Vec<_>>(),
            )
            .await?;

        // Run pre-analyzers to get additional context
        let analyzer_chunks = plugin_manager
            .run_pre_analyzers(diff, &repo_path_str)
            .await?;
        context_chunks.extend(analyzer_chunks);

        // Extract symbols from diff and fetch their definitions
        let symbols = extract_symbols_from_diff(diff);
        if !symbols.is_empty() {
            let definition_chunks = context_fetcher
                .fetch_related_definitions(&diff.file_path, &symbols)
                .await?;
            context_chunks.extend(definition_chunks);
            if let Some(index) = &symbol_index {
                let index_chunks = context_fetcher
                    .fetch_related_definitions_with_index(
                        &diff.file_path,
                        &symbols,
                        index,
                        config.symbol_index_max_locations,
                    )
                    .await?;
                context_chunks.extend(index_chunks);
            }
        }

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

            if !pc.extra_context.is_empty() {
                let extra_chunks = context_fetcher
                    .fetch_additional_context(&pc.extra_context)
                    .await?;
                context_chunks.extend(extra_chunks);
            }
        }

        if let Some(guidance) = build_review_guidance(&config, path_config) {
            local_prompt_config.system_prompt.push_str("\n\n");
            local_prompt_config.system_prompt.push_str(&guidance);
        }

        let local_prompt_builder = core::PromptBuilder::new(local_prompt_config);
        let (system_prompt, user_prompt) =
            local_prompt_builder.build_prompt(&diff, &context_chunks)?;

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
                        if format!("{:?}", comment.category).to_lowercase()
                            == category.to_lowercase()
                        {
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

            let comments = filter_comments_for_diff(&diff, comments);
            all_comments.extend(comments);
        }
    }

    let processed_comments = plugin_manager
        .run_post_processors(all_comments, &repo_path_str)
        .await?;
    let processed_comments = apply_confidence_threshold(processed_comments, config.min_confidence);
    let processed_comments = apply_feedback_suppression(processed_comments, &feedback);
    let processed_comments = apply_feedback_suppression(processed_comments, &feedback);
    let processed_comments = apply_feedback_suppression(processed_comments, &feedback);

    let effective_format = if patch { OutputFormat::Patch } else { format };
    output_comments(&processed_comments, output_path, effective_format).await?;

    Ok(())
}

async fn check_command(path: PathBuf, config: config::Config, format: OutputFormat) -> Result<()> {
    info!("Checking repository at: {}", path.display());
    info!("Using model: {}", config.model);

    let git = core::GitIntegration::new(&path)?;
    let diff_content = git.get_uncommitted_diff()?;
    if diff_content.is_empty() {
        println!("No changes found in {}", path.display());
        return Ok(());
    }

    let repo_root = git.workdir().unwrap_or(path);
    review_diff_content_with_repo(&diff_content, config, format, &repo_root).await
}

async fn git_command(
    command: GitCommands,
    config: config::Config,
    format: OutputFormat,
) -> Result<()> {
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
            let base_branch = base.unwrap_or_else(|| {
                git.get_default_branch()
                    .unwrap_or_else(|_| "main".to_string())
            });
            info!("Analyzing changes from branch: {}", base_branch);
            git.get_branch_diff(&base_branch)?
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

    let repo_root = git.workdir().unwrap_or_else(|| PathBuf::from("."));
    review_diff_content_with_repo(&diff_content, config, format, &repo_root).await
}

async fn pr_command(
    number: Option<u32>,
    repo: Option<String>,
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
        let mut args = vec![
            "pr".to_string(),
            "view".to_string(),
            "--json".to_string(),
            "number".to_string(),
            "-q".to_string(),
            ".number".to_string(),
        ];
        if let Some(repo) = repo.as_ref() {
            args.push("--repo".to_string());
            args.push(repo.clone());
        }

        let output = Command::new("gh").args(&args).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gh pr view failed: {}", stderr.trim());
        }

        let pr_number = String::from_utf8(output.stdout)?.trim().to_string();
        if pr_number.is_empty() {
            anyhow::bail!("Unable to determine PR number from gh output");
        }
        pr_number
    };

    info!("Reviewing PR #{}", pr_number);

    // Get additional git context
    let git = core::GitIntegration::new(".")?;
    let repo_root = git.workdir().unwrap_or_else(|| PathBuf::from("."));
    if let Ok(branch) = git.get_current_branch() {
        info!("Current branch: {}", branch);
    }
    if let Ok(Some(remote)) = git.get_remote_url() {
        info!("Remote URL: {}", remote);
    }

    // Get PR diff
    let mut diff_args = vec!["pr".to_string(), "diff".to_string(), pr_number.clone()];
    if let Some(repo) = repo.as_ref() {
        diff_args.push("--repo".to_string());
        diff_args.push(repo.clone());
    }
    let diff_output = Command::new("gh").args(&diff_args).output()?;
    if !diff_output.status.success() {
        let stderr = String::from_utf8_lossy(&diff_output.stderr);
        anyhow::bail!("gh pr diff failed: {}", stderr.trim());
    }

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
            openai_use_responses: config.openai_use_responses,
        };

        let adapter = adapters::llm::create_adapter(&model_config)?;
        let pr_summary = core::PRSummaryGenerator::generate_summary(&diffs, &git, &adapter).await?;

        println!("{}", pr_summary.to_markdown());
        return Ok(());
    }

    let comments = review_diff_content_raw(&diff_content, config.clone(), &repo_root).await?;

    if post_comments && !comments.is_empty() {
        info!("Posting {} comments to PR", comments.len());

        for comment in &comments {
            let body = format!(
                "**{}**: {}",
                format!("{:?}", comment.severity),
                comment.content
            );

            let mut comment_args = vec![
                "pr".to_string(),
                "comment".to_string(),
                pr_number.clone(),
                "--body".to_string(),
                body,
            ];
            if let Some(repo) = repo.as_ref() {
                comment_args.push("--repo".to_string());
                comment_args.push(repo.clone());
            }
            let comment_output = Command::new("gh").args(&comment_args).output()?;
            if !comment_output.status.success() {
                let stderr = String::from_utf8_lossy(&comment_output.stderr);
                anyhow::bail!("gh pr comment failed: {}", stderr.trim());
            }
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
        openai_use_responses: config.openai_use_responses,
    };

    let adapter = adapters::llm::create_adapter(&model_config)?;

    let (system_prompt, user_prompt) =
        core::CommitPromptBuilder::build_commit_prompt(&diff_content);

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
        println!(
            "\n⚠️  Warning: Commit message exceeds 72 characters ({})",
            commit_message.len()
        );
    }

    Ok(())
}

async fn suggest_pr_title(config: config::Config) -> Result<()> {
    let git = core::GitIntegration::new(".")?;
    let base_branch = git
        .get_default_branch()
        .unwrap_or_else(|_| "main".to_string());
    let diff_content = git.get_branch_diff(&base_branch)?;

    if diff_content.is_empty() {
        println!("No changes found compared to {} branch.", base_branch);
        return Ok(());
    }

    let model_config = adapters::llm::ModelConfig {
        model_name: config.model.clone(),
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
        openai_use_responses: config.openai_use_responses,
    };

    let adapter = adapters::llm::create_adapter(&model_config)?;

    let (system_prompt, user_prompt) =
        core::CommitPromptBuilder::build_pr_title_prompt(&diff_content);

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
        response
            .content
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string()
    };

    println!("\nSuggested PR title:");
    println!("{}", title);

    if title.len() > 65 {
        println!(
            "\n⚠️  Warning: PR title exceeds 65 characters ({})",
            title.len()
        );
    }

    Ok(())
}

async fn compare_command(
    old_file: PathBuf,
    new_file: PathBuf,
    config: config::Config,
    format: OutputFormat,
) -> Result<()> {
    info!(
        "Comparing files: {} vs {}",
        old_file.display(),
        new_file.display()
    );

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
            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
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
    review_diff_content_with_repo(diff_content, config, format, Path::new(".")).await
}

async fn review_diff_content_with_repo(
    diff_content: &str,
    config: config::Config,
    format: OutputFormat,
    repo_path: &Path,
) -> Result<()> {
    let comments = review_diff_content_raw(diff_content, config, repo_path).await?;
    output_comments(&comments, None, format).await
}

async fn review_diff_content_raw(
    diff_content: &str,
    config: config::Config,
    repo_path: &Path,
) -> Result<Vec<core::Comment>> {
    let diffs = core::DiffParser::parse_unified_diff(diff_content)?;
    info!("Parsed {} file diffs", diffs.len());
    let symbol_index = build_symbol_index(&config, repo_path);

    // Initialize plugin manager and load builtin plugins
    let mut plugin_manager = plugins::plugin::PluginManager::new();
    plugin_manager.load_builtin_plugins(&config.plugins).await?;

    let model_config = adapters::llm::ModelConfig {
        model_name: config.model.clone(),
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
        openai_use_responses: config.openai_use_responses,
    };

    let adapter = adapters::llm::create_adapter(&model_config)?;
    let mut base_prompt_config = core::prompt::PromptConfig::default();
    base_prompt_config.max_context_chars = config.max_context_chars;
    base_prompt_config.max_diff_chars = config.max_diff_chars;
    let mut all_comments = Vec::new();

    let repo_path_str = repo_path.to_string_lossy().to_string();
    let context_fetcher = core::ContextFetcher::new(repo_path.to_path_buf());

    for diff in &diffs {
        // Check if file should be excluded
        if config.should_exclude(&diff.file_path) {
            info!("Skipping excluded file: {}", diff.file_path.display());
            continue;
        }
        if diff.is_deleted {
            info!("Skipping deleted file: {}", diff.file_path.display());
            continue;
        }
        if diff.is_binary || diff.hunks.is_empty() {
            info!("Skipping non-text diff: {}", diff.file_path.display());
            continue;
        }

        let mut context_chunks = context_fetcher
            .fetch_context_for_file(
                &diff.file_path,
                &diff
                    .hunks
                    .iter()
                    .map(|h| (h.new_start, h.new_start + h.new_lines.saturating_sub(1)))
                    .collect::<Vec<_>>(),
            )
            .await?;

        // Run pre-analyzers to get additional context
        let analyzer_chunks = plugin_manager
            .run_pre_analyzers(diff, &repo_path_str)
            .await?;
        context_chunks.extend(analyzer_chunks);

        // Extract symbols from diff and fetch their definitions
        let symbols = extract_symbols_from_diff(diff);
        if !symbols.is_empty() {
            let definition_chunks = context_fetcher
                .fetch_related_definitions(&diff.file_path, &symbols)
                .await?;
            context_chunks.extend(definition_chunks);
            if let Some(index) = &symbol_index {
                let index_chunks = context_fetcher
                    .fetch_related_definitions_with_index(
                        &diff.file_path,
                        &symbols,
                        index,
                        config.symbol_index_max_locations,
                    )
                    .await?;
                context_chunks.extend(index_chunks);
            }
        }

        // Get path-specific configuration
        let path_config = config.get_path_config(&diff.file_path);

        // Add focus areas and extra context if configured
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
            if !pc.extra_context.is_empty() {
                let extra_chunks = context_fetcher
                    .fetch_additional_context(&pc.extra_context)
                    .await?;
                context_chunks.extend(extra_chunks);
            }
        }

        // Create prompt builder with config
        let mut local_prompt_config = base_prompt_config.clone();
        if let Some(custom_prompt) = &config.system_prompt {
            local_prompt_config.system_prompt = custom_prompt.clone();
        }
        if let Some(pc) = path_config {
            if let Some(ref prompt) = pc.system_prompt {
                local_prompt_config.system_prompt = prompt.clone();
            }
        }
        if let Some(guidance) = build_review_guidance(&config, path_config) {
            local_prompt_config.system_prompt.push_str("\n\n");
            local_prompt_config.system_prompt.push_str(&guidance);
        }
        let local_prompt_builder = core::PromptBuilder::new(local_prompt_config);
        let (system_prompt, user_prompt) =
            local_prompt_builder.build_prompt(&diff, &context_chunks)?;

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
                        if format!("{:?}", comment.category).to_lowercase()
                            == category.to_lowercase()
                        {
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

            let comments = filter_comments_for_diff(&diff, comments);
            all_comments.extend(comments);
        }
    }

    // Run post-processors to filter and refine comments
    let processed_comments = plugin_manager
        .run_post_processors(all_comments, &repo_path_str)
        .await?;
    let processed_comments = apply_confidence_threshold(processed_comments, config.min_confidence);

    Ok(processed_comments)
}

fn parse_llm_response(
    content: &str,
    file_path: &PathBuf,
) -> Result<Vec<core::comment::RawComment>> {
    let mut comments = Vec::new();
    static LINE_PATTERN: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)line\s+(\d+):\s*(.+)").unwrap());

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip empty lines and common non-issue lines
        if trimmed.is_empty()
            || trimmed.starts_with("```")
            || trimmed.starts_with('#')
            || trimmed.starts_with('<')
            || trimmed.contains("Here are")
            || trimmed.contains("Here is")
            || trimmed.contains("review of")
        {
            continue;
        }

        if let Some(caps) = LINE_PATTERN.captures(line) {
            let line_number: usize = caps.get(1).unwrap().as_str().parse()?;
            let comment_text = caps.get(2).unwrap().as_str().trim();

            // Extract suggestion if present
            let (content, suggestion) = if let Some(sugg_idx) = comment_text.rfind(". Consider ") {
                (
                    comment_text[..sugg_idx + 1].to_string(),
                    Some(
                        comment_text[sugg_idx + 11..]
                            .trim_end_matches('.')
                            .to_string(),
                    ),
                )
            } else if let Some(sugg_idx) = comment_text.rfind(". Use ") {
                (
                    comment_text[..sugg_idx + 1].to_string(),
                    Some(
                        comment_text[sugg_idx + 6..]
                            .trim_end_matches('.')
                            .to_string(),
                    ),
                )
            } else {
                (comment_text.to_string(), None)
            };

            comments.push(core::comment::RawComment {
                file_path: file_path.clone(),
                line_number,
                content,
                suggestion,
                severity: None,
                category: None,
                confidence: None,
                fix_effort: None,
                tags: Vec::new(),
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
        if let Some(suggestion) = &comment.suggestion {
            output.push_str(&format!("# Suggestion: {}\n", suggestion));
        }
    }
    output
}

fn format_as_markdown(comments: &[core::Comment]) -> String {
    let mut output = String::new();

    // Generate summary
    let summary = core::CommentSynthesizer::generate_summary(comments);

    output.push_str("# Code Review Results\n\n");
    output.push_str(&format!("## Summary\n\n"));
    output.push_str(&format!(
        "📊 **Overall Score:** {:.1}/10\n",
        summary.overall_score
    ));
    output.push_str(&format!(
        "📝 **Total Issues:** {}\n",
        summary.total_comments
    ));
    output.push_str(&format!(
        "🚨 **Critical Issues:** {}\n",
        summary.critical_issues
    ));
    output.push_str(&format!(
        "📁 **Files Reviewed:** {}\n\n",
        summary.files_reviewed
    ));

    // Severity breakdown
    output.push_str("### Issues by Severity\n\n");
    let severity_order = ["Error", "Warning", "Info", "Suggestion"];
    for severity in severity_order {
        let count = summary.by_severity.get(severity).copied().unwrap_or(0);
        if count == 0 {
            continue;
        }
        let emoji = match severity {
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
    let category_order = [
        "Security",
        "Performance",
        "Bug",
        "Maintainability",
        "Testing",
        "Style",
        "Documentation",
        "Architecture",
        "BestPractice",
    ];
    for category in category_order {
        let count = summary.by_category.get(category).copied().unwrap_or(0);
        if count == 0 {
            continue;
        }
        let emoji = match category {
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
        comments_by_file
            .entry(&comment.file_path)
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
                comment.line_number, severity_emoji, comment.category
            ));

            output.push_str(&format!(
                "**Confidence:** {:.0}%\n",
                comment.confidence * 100.0
            ));
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
                    if i > 0 {
                        output.push_str(", ");
                    }
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
    info!(
        "Starting smart review analysis with model: {}",
        config.model
    );

    let repo_root = core::GitIntegration::new(".")
        .ok()
        .and_then(|git| git.workdir())
        .unwrap_or_else(|| PathBuf::from("."));
    let repo_path_str = repo_root.to_string_lossy().to_string();
    let context_fetcher = core::ContextFetcher::new(repo_root.clone());

    let mut plugin_manager = plugins::plugin::PluginManager::new();
    plugin_manager.load_builtin_plugins(&config.plugins).await?;

    let diff_content = if let Some(path) = diff_path {
        tokio::fs::read_to_string(path).await?
    } else if std::io::stdin().is_terminal() {
        if let Ok(git) = core::GitIntegration::new(".") {
            let diff = git.get_uncommitted_diff()?;
            if diff.is_empty() {
                println!("No changes found");
                return Ok(());
            }
            diff
        } else {
            println!("No diff provided and not in a git repository.");
            return Ok(());
        }
    } else {
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        buffer
    };

    let diffs = core::DiffParser::parse_unified_diff(&diff_content)?;
    info!("Parsed {} file diffs", diffs.len());
    let walkthrough = build_change_walkthrough(&diffs);
    let symbol_index = build_symbol_index(&config, &repo_root);

    let model_config = adapters::llm::ModelConfig {
        model_name: config.model.clone(),
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
        openai_use_responses: config.openai_use_responses,
    };

    let adapter = adapters::llm::create_adapter(&model_config)?;
    let mut all_comments = Vec::new();
    let pr_summary = if config.smart_review_summary {
        match core::GitIntegration::new(&repo_root) {
            Ok(git) => {
                let options = core::SummaryOptions {
                    include_diagram: config.smart_review_diagram,
                };
                match core::PRSummaryGenerator::generate_summary_with_options(
                    &diffs, &git, &adapter, options,
                )
                .await
                {
                    Ok(summary) => Some(summary),
                    Err(err) => {
                        warn!("PR summary generation failed: {}", err);
                        None
                    }
                }
            }
            Err(err) => {
                warn!("Skipping PR summary (git unavailable): {}", err);
                None
            }
        }
    } else {
        None
    };

    for diff in &diffs {
        // Check if file should be excluded
        if config.should_exclude(&diff.file_path) {
            info!("Skipping excluded file: {}", diff.file_path.display());
            continue;
        }
        if diff.is_deleted {
            info!("Skipping deleted file: {}", diff.file_path.display());
            continue;
        }
        if diff.is_binary || diff.hunks.is_empty() {
            info!("Skipping non-text diff: {}", diff.file_path.display());
            continue;
        }

        let mut context_chunks = context_fetcher
            .fetch_context_for_file(
                &diff.file_path,
                &diff
                    .hunks
                    .iter()
                    .map(|h| (h.new_start, h.new_start + h.new_lines.saturating_sub(1)))
                    .collect::<Vec<_>>(),
            )
            .await?;

        // Run pre-analyzers to get additional context
        let analyzer_chunks = plugin_manager
            .run_pre_analyzers(diff, &repo_path_str)
            .await?;
        context_chunks.extend(analyzer_chunks);

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
            if !pc.extra_context.is_empty() {
                let extra_chunks = context_fetcher
                    .fetch_additional_context(&pc.extra_context)
                    .await?;
                context_chunks.extend(extra_chunks);
            }
        }

        // Extract symbols and get definitions
        let symbols = extract_symbols_from_diff(diff);
        if !symbols.is_empty() {
            let definition_chunks = context_fetcher
                .fetch_related_definitions(&diff.file_path, &symbols)
                .await?;
            context_chunks.extend(definition_chunks);
            if let Some(index) = &symbol_index {
                let index_chunks = context_fetcher
                    .fetch_related_definitions_with_index(
                        &diff.file_path,
                        &symbols,
                        index,
                        config.symbol_index_max_locations,
                    )
                    .await?;
                context_chunks.extend(index_chunks);
            }
        }

        let guidance = build_review_guidance(&config, path_config);
        let (system_prompt, user_prompt) =
            core::SmartReviewPromptBuilder::build_enhanced_review_prompt(
                diff,
                &context_chunks,
                config.max_context_chars,
                config.max_diff_chars,
                guidance.as_deref(),
            )?;

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
                        if format!("{:?}", comment.category).to_lowercase()
                            == category.to_lowercase()
                        {
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

            let comments = filter_comments_for_diff(diff, comments);
            all_comments.extend(comments);
        }
    }

    // Run post-processors to filter and refine comments
    let processed_comments = plugin_manager
        .run_post_processors(all_comments, &repo_path_str)
        .await?;
    let processed_comments = apply_confidence_threshold(processed_comments, config.min_confidence);

    // Generate summary and output results
    let summary = core::CommentSynthesizer::generate_summary(&processed_comments);
    let output = format_smart_review_output(
        &processed_comments,
        &summary,
        pr_summary.as_ref(),
        &walkthrough,
    );

    if let Some(path) = output_path {
        tokio::fs::write(path, output).await?;
    } else {
        println!("{}", output);
    }

    Ok(())
}

fn parse_smart_review_response(
    content: &str,
    file_path: &PathBuf,
) -> Result<Vec<core::comment::RawComment>> {
    let mut comments = Vec::new();
    let mut current_comment: Option<core::comment::RawComment> = None;
    let mut section: Option<SmartSection> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        if let Some(title) = trimmed.strip_prefix("ISSUE:") {
            // Save previous comment if exists
            if let Some(comment) = current_comment.take() {
                comments.push(comment);
            }

            // Start new comment
            let title = title.trim();
            current_comment = Some(core::comment::RawComment {
                file_path: file_path.clone(),
                line_number: 1,
                content: title.to_string(),
                suggestion: None,
                severity: None,
                category: None,
                confidence: None,
                fix_effort: None,
                tags: Vec::new(),
            });
            section = None;
            continue;
        }

        let comment = match current_comment.as_mut() {
            Some(comment) => comment,
            None => continue,
        };

        if let Some(value) = trimmed.strip_prefix("LINE:") {
            if let Ok(line_num) = value.trim().parse::<usize>() {
                comment.line_number = line_num;
            }
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("SEVERITY:") {
            comment.severity = parse_smart_severity(value.trim());
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("CATEGORY:") {
            comment.category = parse_smart_category(value.trim());
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("CONFIDENCE:") {
            comment.confidence = parse_smart_confidence(value.trim());
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("EFFORT:") {
            comment.fix_effort = parse_smart_effort(value.trim());
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("TAGS:") {
            comment.tags = parse_smart_tags(value.trim());
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("DESCRIPTION:") {
            section = Some(SmartSection::Description);
            let value = value.trim();
            if !value.is_empty() {
                append_content(&mut comment.content, value);
            }
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("SUGGESTION:") {
            section = Some(SmartSection::Suggestion);
            let value = value.trim();
            if !value.is_empty() {
                append_suggestion(&mut comment.suggestion, value);
            }
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        match section {
            Some(SmartSection::Suggestion) => append_suggestion(&mut comment.suggestion, trimmed),
            _ => append_content(&mut comment.content, trimmed),
        }
    }

    // Save last comment
    if let Some(comment) = current_comment {
        comments.push(comment);
    }

    Ok(comments)
}

#[derive(Clone, Copy)]
enum SmartSection {
    Description,
    Suggestion,
}

fn append_content(content: &mut String, value: &str) {
    if !content.is_empty() {
        content.push(' ');
    }
    content.push_str(value);
}

fn append_suggestion(suggestion: &mut Option<String>, value: &str) {
    match suggestion {
        Some(existing) => {
            if !existing.is_empty() {
                existing.push(' ');
            }
            existing.push_str(value);
        }
        None => {
            *suggestion = Some(value.to_string());
        }
    }
}

fn parse_smart_severity(value: &str) -> Option<core::comment::Severity> {
    match value.to_lowercase().as_str() {
        "critical" => Some(core::comment::Severity::Error),
        "high" => Some(core::comment::Severity::Warning),
        "medium" => Some(core::comment::Severity::Info),
        "low" => Some(core::comment::Severity::Suggestion),
        _ => None,
    }
}

fn parse_smart_category(value: &str) -> Option<core::comment::Category> {
    match value.to_lowercase().as_str() {
        "security" => Some(core::comment::Category::Security),
        "performance" => Some(core::comment::Category::Performance),
        "bug" => Some(core::comment::Category::Bug),
        "maintainability" => Some(core::comment::Category::Maintainability),
        "testing" => Some(core::comment::Category::Testing),
        "style" => Some(core::comment::Category::Style),
        "documentation" => Some(core::comment::Category::Documentation),
        "architecture" => Some(core::comment::Category::Architecture),
        "bestpractice" | "best_practice" | "best practice" => {
            Some(core::comment::Category::BestPractice)
        }
        _ => None,
    }
}

fn parse_smart_confidence(value: &str) -> Option<f32> {
    let trimmed = value.trim().trim_end_matches('%');
    if let Ok(percent) = trimmed.parse::<f32>() {
        Some((percent / 100.0).max(0.0).min(1.0))
    } else {
        None
    }
}

fn parse_smart_effort(value: &str) -> Option<core::comment::FixEffort> {
    match value.to_lowercase().as_str() {
        "low" => Some(core::comment::FixEffort::Low),
        "medium" => Some(core::comment::FixEffort::Medium),
        "high" => Some(core::comment::FixEffort::High),
        _ => None,
    }
}

fn parse_smart_tags(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .map(|tag| tag.to_string())
        .collect()
}

fn format_smart_review_output(
    comments: &[core::Comment],
    summary: &core::comment::ReviewSummary,
    pr_summary: Option<&core::pr_summary::PRSummary>,
    walkthrough: &str,
) -> String {
    let mut output = String::new();

    output.push_str("# 🤖 Smart Review Analysis Results\n\n");

    // Executive Summary
    output.push_str("## 📊 Executive Summary\n\n");
    let score_emoji = if summary.overall_score >= 8.0 {
        "🟢"
    } else if summary.overall_score >= 6.0 {
        "🟡"
    } else {
        "🔴"
    };
    output.push_str(&format!(
        "{} **Code Quality Score:** {:.1}/10\n",
        score_emoji, summary.overall_score
    ));
    output.push_str(&format!(
        "📝 **Total Issues Found:** {}\n",
        summary.total_comments
    ));
    output.push_str(&format!(
        "🚨 **Critical Issues:** {}\n",
        summary.critical_issues
    ));
    output.push_str(&format!(
        "📁 **Files Analyzed:** {}\n\n",
        summary.files_reviewed
    ));

    if let Some(pr_summary) = pr_summary {
        output.push_str(&format_pr_summary_section(pr_summary));
        output.push('\n');
    }

    if !walkthrough.trim().is_empty() {
        output.push_str(walkthrough);
        output.push('\n');
    }

    // Quick Stats
    output.push_str("### 📈 Issue Breakdown\n\n");

    output.push_str("#### By Severity\n\n");
    output.push_str("| Severity | Count |\n");
    output.push_str("|----------|-------|\n");
    let severities = ["Error", "Warning", "Info", "Suggestion"];
    for severity in severities {
        let sev_count = summary.by_severity.get(severity).unwrap_or(&0);
        output.push_str(&format!("| {} | {} |\n", severity, sev_count));
    }
    output.push_str("\n");

    output.push_str("#### By Category\n\n");
    output.push_str("| Category | Count |\n");
    output.push_str("|----------|-------|\n");
    let categories = [
        "Security",
        "Performance",
        "Bug",
        "Maintainability",
        "Testing",
        "Style",
        "Documentation",
        "Architecture",
        "BestPractice",
    ];
    for category in categories {
        let cat_count = summary.by_category.get(category).unwrap_or(&0);
        output.push_str(&format!("| {} | {} |\n", category, cat_count));
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

    if comment.tags.is_empty() {
        output.push_str(&format!(
            "**Confidence:** {:.0}%\n\n",
            comment.confidence * 100.0
        ));
    } else {
        output.push_str(&format!(
            "**Confidence:** {:.0}% | **Tags:** ",
            comment.confidence * 100.0
        ));
        for (i, tag) in comment.tags.iter().enumerate() {
            if i > 0 {
                output.push_str(", ");
            }
            output.push_str(&format!("`{}`", tag));
        }
        output.push_str("\n\n");
    }

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

async fn feedback_command(
    config: config::Config,
    accept: Option<PathBuf>,
    reject: Option<PathBuf>,
    feedback_path: Option<PathBuf>,
) -> Result<()> {
    let (action, input_path) = match (accept, reject) {
        (Some(path), None) => ("accept", path),
        (None, Some(path)) => ("reject", path),
        _ => {
            anyhow::bail!("Specify exactly one of --accept or --reject");
        }
    };

    let feedback_path = feedback_path.unwrap_or_else(|| config.feedback_path.clone());
    let content = tokio::fs::read_to_string(&input_path).await?;
    let mut comments: Vec<core::Comment> = serde_json::from_str(&content)?;

    for comment in &mut comments {
        if comment.id.trim().is_empty() {
            comment.id = core::comment::compute_comment_id(
                &comment.file_path,
                &comment.content,
                &comment.category,
            );
        }
    }

    let mut store = load_feedback_store_from_path(&feedback_path);
    let mut updated = 0usize;

    if action == "accept" {
        for comment in &comments {
            if store.accept.insert(comment.id.clone()) {
                updated += 1;
            }
            store.suppress.remove(&comment.id);
        }
    } else {
        for comment in &comments {
            if store.suppress.insert(comment.id.clone()) {
                updated += 1;
            }
            store.accept.remove(&comment.id);
        }
    }

    save_feedback_store(&feedback_path, &store)?;
    println!(
        "Updated feedback store at {} ({} {} comment(s))",
        feedback_path.display(),
        updated,
        action
    );

    Ok(())
}

fn extract_symbols_from_diff(diff: &core::UnifiedDiff) -> Vec<String> {
    let mut symbols = Vec::new();
    static SYMBOL_REGEX: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\b([A-Z][a-zA-Z0-9_]*|[a-z][a-zA-Z0-9_]*)\s*\(").unwrap());
    static CLASS_REGEX: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b(class|struct|interface|enum)\s+([A-Z][a-zA-Z0-9_]*)").unwrap()
    });

    for hunk in &diff.hunks {
        for line in &hunk.changes {
            if matches!(
                line.change_type,
                core::diff_parser::ChangeType::Added | core::diff_parser::ChangeType::Removed
            ) {
                // Extract function calls and references
                for capture in SYMBOL_REGEX.captures_iter(&line.content) {
                    if let Some(symbol) = capture.get(1) {
                        let symbol_str = symbol.as_str().to_string();
                        if symbol_str.len() > 2 && !symbols.contains(&symbol_str) {
                            symbols.push(symbol_str);
                        }
                    }
                }

                // Also look for class/struct references
                for capture in CLASS_REGEX.captures_iter(&line.content) {
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

fn filter_comments_for_diff(
    diff: &core::UnifiedDiff,
    comments: Vec<core::Comment>,
) -> Vec<core::Comment> {
    let mut filtered = Vec::new();
    let total = comments.len();
    for comment in comments {
        if is_line_in_diff(diff, comment.line_number) {
            filtered.push(comment);
        }
    }

    if filtered.len() != total {
        let dropped = total.saturating_sub(filtered.len());
        info!(
            "Dropped {} comment(s) for {} due to unmatched line numbers",
            dropped,
            diff.file_path.display()
        );
    }

    filtered
}

fn build_review_guidance(
    config: &config::Config,
    path_config: Option<&config::PathConfig>,
) -> Option<String> {
    let mut sections = Vec::new();

    if let Some(profile) = config.review_profile.as_deref() {
        let guidance = match profile {
            "chill" => Some(
                "Be conservative and only surface high-confidence, high-impact issues. Avoid nitpicks and redundant comments.",
            ),
            "assertive" => Some(
                "Be thorough and proactive. Surface edge cases, latent risks, and maintainability concerns even if they are subtle.",
            ),
            _ => None,
        };
        if let Some(text) = guidance {
            sections.push(format!("Review profile ({}): {}", profile, text));
        }
    }

    if let Some(instructions) = config.review_instructions.as_deref() {
        let trimmed = instructions.trim();
        if !trimmed.is_empty() {
            sections.push(format!("Global review instructions:\n{}", trimmed));
        }
    }

    if let Some(pc) = path_config {
        if let Some(instructions) = pc.review_instructions.as_deref() {
            let trimmed = instructions.trim();
            if !trimmed.is_empty() {
                sections.push(format!("Path-specific instructions:\n{}", trimmed));
            }
        }
    }

    if sections.is_empty() {
        None
    } else {
        Some(format!(
            "Additional review guidance:\n{}",
            sections.join("\n\n")
        ))
    }
}

fn build_change_walkthrough(diffs: &[core::UnifiedDiff]) -> String {
    let mut entries = Vec::new();
    let mut truncated = false;
    let max_entries = 50usize;

    for diff in diffs {
        if diff.is_binary {
            continue;
        }

        let mut added = 0usize;
        let mut removed = 0usize;
        for hunk in &diff.hunks {
            for change in &hunk.changes {
                match change.change_type {
                    core::diff_parser::ChangeType::Added => added += 1,
                    core::diff_parser::ChangeType::Removed => removed += 1,
                    _ => {}
                }
            }
        }

        let status = if diff.is_deleted {
            "deleted"
        } else if diff.is_new {
            "new"
        } else {
            "modified"
        };

        entries.push(format!(
            "- `{}` ({}; +{}, -{})",
            diff.file_path.display(),
            status,
            added,
            removed
        ));

        if entries.len() >= max_entries {
            truncated = true;
            break;
        }
    }

    if entries.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    output.push_str("## 🧭 Change Walkthrough\n\n");
    output.push_str(&entries.join("\n"));
    output.push('\n');
    if truncated {
        output.push_str("\n...truncated (too many files)\n");
    }

    output
}

fn build_symbol_index(config: &config::Config, repo_root: &Path) -> Option<core::SymbolIndex> {
    if !config.symbol_index {
        return None;
    }

    let should_exclude = |path: &PathBuf| config.should_exclude(path);
    match core::SymbolIndex::build(
        repo_root,
        config.symbol_index_max_files,
        config.symbol_index_max_bytes,
        config.symbol_index_max_locations,
        should_exclude,
    ) {
        Ok(index) => {
            info!(
                "Indexed {} symbols across {} files",
                index.symbols_indexed(),
                index.files_indexed()
            );
            Some(index)
        }
        Err(err) => {
            warn!("Symbol index build failed: {}", err);
            None
        }
    }
}

fn format_pr_summary_section(summary: &core::pr_summary::PRSummary) -> String {
    let mut output = String::new();
    output.push_str("## 🧾 PR Summary\n\n");
    output.push_str(&format!(
        "**{}** ({:?})\n\n",
        summary.title, summary.change_type
    ));

    if !summary.description.is_empty() {
        output.push_str(&format!("{}\n\n", summary.description));
    }

    if !summary.key_changes.is_empty() {
        output.push_str("### Key Changes\n\n");
        for change in &summary.key_changes {
            output.push_str(&format!("- {}\n", change));
        }
        output.push('\n');
    }

    if let Some(breaking) = &summary.breaking_changes {
        output.push_str("### Breaking Changes\n\n");
        output.push_str(&format!("{}\n\n", breaking));
    }

    if !summary.testing_notes.is_empty() {
        output.push_str("### Testing Notes\n\n");
        output.push_str(&format!("{}\n\n", summary.testing_notes));
    }

    if let Some(diagram) = &summary.visual_diff {
        if !diagram.trim().is_empty() {
            output.push_str("### Diagram\n\n");
            output.push_str("```mermaid\n");
            output.push_str(diagram.trim());
            output.push_str("\n```\n\n");
        }
    }

    output
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FeedbackStore {
    #[serde(default)]
    suppress: HashSet<String>,
    #[serde(default)]
    accept: HashSet<String>,
}

fn load_feedback_store_from_path(path: &Path) -> FeedbackStore {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => FeedbackStore::default(),
    }
}

fn load_feedback_store(config: &config::Config) -> FeedbackStore {
    load_feedback_store_from_path(&config.feedback_path)
}

fn save_feedback_store(path: &Path, store: &FeedbackStore) -> Result<()> {
    let content = serde_json::to_string_pretty(store)?;
    std::fs::write(path, content)?;
    Ok(())
}

fn apply_feedback_suppression(
    comments: Vec<core::Comment>,
    feedback: &FeedbackStore,
) -> Vec<core::Comment> {
    if feedback.suppress.is_empty() {
        return comments;
    }

    let total = comments.len();
    let mut kept = Vec::with_capacity(total);

    for comment in comments {
        if feedback.suppress.contains(&comment.id) {
            continue;
        }
        kept.push(comment);
    }

    if kept.len() != total {
        let dropped = total.saturating_sub(kept.len());
        info!(
            "Dropped {} comment(s) due to feedback suppression rules",
            dropped
        );
    }

    kept
}

fn apply_confidence_threshold(
    comments: Vec<core::Comment>,
    min_confidence: f32,
) -> Vec<core::Comment> {
    if min_confidence <= 0.0 {
        return comments;
    }

    let total = comments.len();
    let mut kept = Vec::with_capacity(total);

    for comment in comments {
        if comment.confidence >= min_confidence {
            kept.push(comment);
        }
    }

    if kept.len() != total {
        let dropped = total.saturating_sub(kept.len());
        info!(
            "Dropped {} comment(s) below confidence threshold {}",
            dropped, min_confidence
        );
    }

    kept
}

fn is_line_in_diff(diff: &core::UnifiedDiff, line_number: usize) -> bool {
    if line_number == 0 {
        return false;
    }
    diff.hunks.iter().any(|hunk| {
        hunk.changes
            .iter()
            .any(|line| line.new_line_no == Some(line_number))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_smart_review_response_parses_fields() {
        let input = r#"
ISSUE: Missing auth check
LINE: 42
SEVERITY: CRITICAL
CATEGORY: Security
CONFIDENCE: 85%
EFFORT: High

DESCRIPTION:
Authentication is missing.

SUGGESTION:
Add a guard.

TAGS: auth, security
"#;
        let file_path = PathBuf::from("src/lib.rs");
        let comments = parse_smart_review_response(input, &file_path).unwrap();
        assert_eq!(comments.len(), 1);

        let comment = &comments[0];
        assert_eq!(comment.line_number, 42);
        assert_eq!(comment.severity, Some(core::comment::Severity::Error));
        assert_eq!(comment.category, Some(core::comment::Category::Security));
        assert!(comment.content.contains("Missing auth check"));
        assert!(comment.content.contains("Authentication is missing."));
        assert_eq!(comment.suggestion.as_deref(), Some("Add a guard."));
        assert_eq!(
            comment.tags,
            vec!["auth".to_string(), "security".to_string()]
        );

        let confidence = comment.confidence.unwrap_or(0.0);
        assert!((confidence - 0.85).abs() < 0.0001);
        assert_eq!(comment.fix_effort, Some(core::comment::FixEffort::High));
    }
}
