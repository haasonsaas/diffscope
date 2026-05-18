use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::warn;

/// Identifies the role a model plays in the review pipeline.
///
/// Different tasks benefit from different model tiers: cheap/fast models
/// for triage and summarization, frontier models for deep review, and
/// specialised models for reasoning or embeddings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    /// The main review model (default).
    Primary,
    /// Cheap/fast model for triage, summarization, NL translation.
    Weak,
    /// Reasoning-capable model for complex analysis and self-reflection.
    Reasoning,
    /// Embedding model for RAG indexing.
    Embedding,
    /// Fast model for lightweight LLM tasks: PR summaries, commit messages,
    /// PR titles, diagram generation. Falls back to Weak, then Primary.
    Fast,
}

impl ModelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Weak => "weak",
            Self::Reasoning => "reasoning",
            Self::Embedding => "embedding",
            Self::Fast => "fast",
        }
    }

    pub fn supports_text_generation(self) -> bool {
        !matches!(self, Self::Embedding)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationConsensusMode {
    Any,
    Majority,
    All,
}

impl VerificationConsensusMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Majority => "majority",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigValidationIssueLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigValidationIssue {
    pub level: ConfigValidationIssueLevel,
    pub field: String,
    pub message: String,
}

impl ConfigValidationIssue {
    fn warning(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: ConfigValidationIssueLevel::Warning,
            field: field.into(),
            message: message.into(),
        }
    }

    fn error(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: ConfigValidationIssueLevel::Error,
            field: field.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultConfig {
    /// HashiCorp Vault server address (e.g., https://vault.example.com:8200).
    #[serde(default, rename = "vault_addr")]
    pub addr: Option<String>,

    /// Vault authentication token.
    #[serde(default, rename = "vault_token")]
    pub token: Option<String>,

    /// Secret path in Vault (e.g., "diffscope" or "ci/diffscope").
    #[serde(default, rename = "vault_path")]
    pub path: Option<String>,

    /// Key within the Vault secret to extract as the API key (default: "api_key").
    #[serde(default, rename = "vault_key")]
    pub key: Option<String>,

    /// Vault KV engine mount point (default: "secret").
    #[serde(default, rename = "vault_mount")]
    pub mount: Option<String>,

    /// Vault Enterprise namespace.
    #[serde(default, rename = "vault_namespace")]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubConfig {
    #[serde(default, rename = "github_token")]
    pub token: Option<String>,

    /// GitHub App ID (from app settings page).
    #[serde(default, rename = "github_app_id")]
    pub app_id: Option<u64>,

    /// GitHub App OAuth client ID (for device flow auth).
    #[serde(default, rename = "github_client_id")]
    pub client_id: Option<String>,

    /// GitHub App OAuth client secret.
    #[serde(default, rename = "github_client_secret")]
    pub client_secret: Option<String>,

    /// GitHub App private key (PEM content).
    #[serde(default, rename = "github_private_key")]
    pub private_key: Option<String>,

    /// Webhook secret for verifying GitHub webhook signatures.
    #[serde(default, rename = "github_webhook_secret")]
    pub webhook_secret: Option<String>,

    /// pull_request actions that should start an automated review.
    ///
    /// Supported values: opened, synchronize, reopened, review_requested.
    #[serde(
        default = "default_github_auto_review_events",
        rename = "github_auto_review_events"
    )]
    pub auto_review_events: Vec<String>,

    /// GitHub user logins that can trigger review_requested automation.
    ///
    /// This is intentionally separate from auto_review_events so an org-level
    /// webhook can listen to pull_request events without reviewing every PR.
    #[serde(default, rename = "github_review_request_reviewers")]
    pub review_request_reviewers: Vec<String>,
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            token: None,
            app_id: None,
            client_id: None,
            client_secret: None,
            private_key: None,
            webhook_secret: None,
            auto_review_events: default_github_auto_review_events(),
            review_request_reviewers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JiraConfig {
    #[serde(default, rename = "jira_base_url")]
    pub base_url: Option<String>,

    #[serde(default, rename = "jira_email")]
    pub email: Option<String>,

    #[serde(default, rename = "jira_api_token")]
    pub api_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinearConfig {
    #[serde(default, rename = "linear_api_key")]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LinkedIssueProvider {
    Jira,
    Linear,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinkedIssueContext {
    pub provider: LinkedIssueProvider,
    pub identifier: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocumentContext {
    pub source: String,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutomationConfig {
    /// Outbound webhook URL for downstream automation consumers.
    #[serde(default, rename = "automation_webhook_url")]
    pub webhook_url: Option<String>,

    /// Optional shared secret for signing outbound automation webhooks.
    #[serde(default, rename = "automation_webhook_secret")]
    pub webhook_secret: Option<String>,
}

pub(crate) const DEFAULT_SERVER_RATE_LIMIT_PER_MINUTE: u32 = 60;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerSecurityConfig {
    /// Shared API key required for protected server mutations when configured.
    #[serde(default, rename = "server_api_key")]
    pub api_key: Option<String>,

    /// Maximum protected API mutations allowed per minute when auth is enabled.
    #[serde(default, rename = "server_rate_limit_per_minute")]
    pub rate_limit_per_minute: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Enable agent loop for iterative tool-calling review (default false).
    #[serde(default, rename = "agent_review")]
    pub enabled: bool,

    /// Maximum number of LLM round-trips in agent mode (default 10).
    #[serde(
        default = "default_agent_max_iterations",
        rename = "agent_max_iterations"
    )]
    pub max_iterations: usize,

    /// Optional total token budget for agent loop.
    #[serde(default, rename = "agent_max_total_tokens")]
    pub max_total_tokens: Option<usize>,

    /// Which agent tools are enabled. None = all tools enabled.
    #[serde(default, rename = "agent_tools_enabled")]
    pub tools_enabled: Option<Vec<String>>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_iterations: default_agent_max_iterations(),
            max_total_tokens: None,
            tools_enabled: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationConfig {
    /// Whether to run the verification pass on review comments (default true).
    #[serde(default = "default_true", rename = "verification_pass")]
    pub enabled: bool,

    /// Which model role to use for the verification pass (default Weak).
    #[serde(
        default = "default_verification_model_role",
        rename = "verification_model_role"
    )]
    pub model_role: ModelRole,

    /// Additional model roles used as verification judges.
    #[serde(
        default = "default_verification_additional_model_roles",
        rename = "verification_additional_model_roles"
    )]
    pub additional_model_roles: Vec<ModelRole>,

    /// How multiple verification judges should be combined.
    #[serde(
        default = "default_verification_consensus_mode",
        rename = "verification_consensus_mode"
    )]
    pub consensus_mode: VerificationConsensusMode,

    /// Minimum verification score to keep a comment (0-10, default 5).
    #[serde(
        default = "default_verification_min_score",
        rename = "verification_min_score"
    )]
    pub min_score: u8,

    /// Maximum number of comments to send through verification (default 20).
    #[serde(
        default = "default_verification_max_comments",
        rename = "verification_max_comments"
    )]
    pub max_comments: usize,

    /// When true, keep original comments if the verification pass fails or
    /// returns an unparseable response (default false).
    #[serde(default = "default_false", rename = "verification_fail_open")]
    pub fail_open: bool,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model_role: default_verification_model_role(),
            additional_model_roles: default_verification_additional_model_roles(),
            consensus_mode: default_verification_consensus_mode(),
            min_score: default_verification_min_score(),
            max_comments: default_verification_max_comments(),
            fail_open: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    #[serde(default = "default_review_retention_max_age_days")]
    pub review_max_age_days: i64,

    #[serde(default = "default_review_retention_max_count")]
    pub review_max_count: usize,

    #[serde(default = "default_eval_artifact_retention_max_age_days")]
    pub eval_artifact_max_age_days: i64,

    #[serde(default = "default_trend_history_max_entries")]
    pub trend_history_max_entries: usize,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            review_max_age_days: default_review_retention_max_age_days(),
            review_max_count: default_review_retention_max_count(),
            eval_artifact_max_age_days: default_eval_artifact_retention_max_age_days(),
            trend_history_max_entries: default_trend_history_max_entries(),
        }
    }
}

impl RetentionConfig {
    pub fn normalize(&mut self) {
        if self.review_max_age_days <= 0 {
            self.review_max_age_days = default_review_retention_max_age_days();
        }
        if self.review_max_count == 0 {
            self.review_max_count = default_review_retention_max_count();
        }
        if self.eval_artifact_max_age_days <= 0 {
            self.eval_artifact_max_age_days = default_eval_artifact_retention_max_age_days();
        }
        if self.trend_history_max_entries == 0 {
            self.trend_history_max_entries = default_trend_history_max_entries();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_model")]
    pub model: String,

    /// Cheap/fast model for triage, summarization, NL translation.
    #[serde(default = "default_model_weak")]
    pub model_weak: Option<String>,

    /// Fast model for lightweight LLM tasks: PR summaries, commit messages,
    /// PR titles, diagram generation. Falls back to model_weak, then primary.
    #[serde(default = "default_model_fast")]
    pub model_fast: Option<String>,

    /// Reasoning-capable model for complex analysis and self-reflection.
    #[serde(default = "default_model_reasoning")]
    pub model_reasoning: Option<String>,

    /// Embedding model for RAG indexing.
    #[serde(default)]
    pub model_embedding: Option<String>,

    /// Which configured model role should generate review findings.
    #[serde(
        default = "default_generation_model_role",
        rename = "generation_model_role"
    )]
    pub generation_model_role: ModelRole,

    /// Which configured model role should run independent auditing tasks.
    #[serde(
        default = "default_auditing_model_role",
        rename = "auditing_model_role"
    )]
    pub auditing_model_role: ModelRole,

    /// Fallback models tried in order when the primary model fails.
    #[serde(default)]
    pub fallback_models: Vec<String>,

    #[serde(default = "default_temperature")]
    pub temperature: f32,

    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,

    #[serde(default = "default_max_context_chars")]
    pub max_context_chars: usize,

    #[serde(default = "default_max_diff_chars")]
    pub max_diff_chars: usize,

    #[serde(default = "default_context_max_chunks")]
    pub context_max_chunks: usize,

    #[serde(default = "default_context_budget_chars")]
    pub context_budget_chars: usize,

    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,

    #[serde(default = "default_strictness")]
    pub strictness: u8,

    #[serde(default = "default_comment_types")]
    pub comment_types: Vec<String>,

    #[serde(default)]
    pub review_profile: Option<String>,

    #[serde(default)]
    pub review_instructions: Option<String>,

    /// Natural language review rules (one per item); injected into prompt as bullets (#12).
    #[serde(default)]
    pub review_rules_prose: Option<Vec<String>>,

    #[serde(default = "default_true")]
    pub smart_review_summary: bool,

    #[serde(default)]
    pub smart_review_diagram: bool,

    #[serde(default = "default_true")]
    pub symbol_index: bool,

    #[serde(default = "default_symbol_index_provider")]
    pub symbol_index_provider: String,

    #[serde(default = "default_symbol_index_max_files")]
    pub symbol_index_max_files: usize,

    #[serde(default = "default_symbol_index_max_bytes")]
    pub symbol_index_max_bytes: usize,

    #[serde(default = "default_symbol_index_max_locations")]
    pub symbol_index_max_locations: usize,

    #[serde(default = "default_symbol_index_graph_hops")]
    pub symbol_index_graph_hops: usize,

    #[serde(default = "default_symbol_index_graph_max_files")]
    pub symbol_index_graph_max_files: usize,

    #[serde(default)]
    pub symbol_index_lsp_command: Option<String>,

    #[serde(default = "default_symbol_index_lsp_languages")]
    pub symbol_index_lsp_languages: HashMap<String, String>,

    /// When true, triage skips deletion-only diffs (#29). Default false (deletions get review).
    #[serde(default)]
    pub triage_skip_deletion_only: bool,

    #[serde(default = "default_feedback_path")]
    pub feedback_path: PathBuf,

    #[serde(default = "default_eval_trend_path")]
    pub eval_trend_path: PathBuf,

    #[serde(default = "default_feedback_eval_trend_path")]
    pub feedback_eval_trend_path: PathBuf,

    #[serde(default)]
    pub retention: RetentionConfig,

    /// Path to the convention store file for learned review patterns.
    /// Defaults to ~/.local/share/diffscope/conventions.json if not set.
    #[serde(default)]
    pub convention_store_path: Option<String>,

    pub system_prompt: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,

    #[serde(default)]
    pub adapter: Option<String>,

    #[serde(default)]
    pub context_window: Option<usize>,

    #[serde(default)]
    pub openai_use_responses: Option<bool>,

    /// HTTP timeout in seconds for LLM adapter requests.
    /// Defaults: 60s for cloud APIs, 300s for local endpoints.
    #[serde(default)]
    pub adapter_timeout_secs: Option<u64>,

    /// Maximum number of retries on transient failures (429, 5xx).
    #[serde(default)]
    pub adapter_max_retries: Option<usize>,

    /// Base delay in milliseconds between retries (linear backoff).
    #[serde(default)]
    pub adapter_retry_delay_ms: Option<u64>,

    /// Maximum number of file changes before skipping review (0 = no limit).
    #[serde(default)]
    pub file_change_limit: Option<usize>,

    /// Auto-detect and absorb .cursorrules, CLAUDE.md, agents.md files.
    #[serde(default = "default_true")]
    pub auto_detect_instructions: bool,

    /// Language/locale for review output (e.g., "en", "ja", "de").
    #[serde(default)]
    pub output_language: Option<String>,

    /// Whether to include AI fix suggestions with comments.
    #[serde(default = "default_true")]
    pub include_fix_suggestions: bool,

    /// Minimum number of rejections before adaptive suppression kicks in.
    #[serde(default = "default_feedback_suppression_threshold")]
    pub feedback_suppression_threshold: usize,

    /// Margin: rejected must exceed accepted by this amount for suppression.
    #[serde(default = "default_feedback_suppression_margin")]
    pub feedback_suppression_margin: usize,

    #[serde(default, flatten)]
    pub vault: VaultConfig,

    #[serde(default)]
    pub plugins: PluginConfig,

    #[serde(default = "default_exclude_patterns")]
    pub exclude_patterns: Vec<String>,

    #[serde(default)]
    pub paths: HashMap<String, PathConfig>,

    #[serde(default)]
    pub custom_context: Vec<CustomContextConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_issue_contexts: Vec<LinkedIssueContext>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub document_contexts: Vec<DocumentContext>,

    #[serde(default)]
    pub pattern_repositories: Vec<PatternRepositoryConfig>,

    #[serde(default)]
    pub rules_files: Vec<String>,

    #[serde(default = "default_max_active_rules")]
    pub max_active_rules: usize,

    #[serde(default)]
    pub rule_priority: Vec<String>,

    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,

    #[serde(default, flatten)]
    pub github: GitHubConfig,

    #[serde(default, flatten)]
    pub jira: JiraConfig,

    #[serde(default, flatten)]
    pub linear: LinearConfig,

    #[serde(default, flatten)]
    pub automation: AutomationConfig,

    #[serde(default, flatten)]
    pub server_security: ServerSecurityConfig,

    /// When true, run separate specialized LLM passes for security, correctness,
    /// and style instead of a single monolithic review prompt.
    #[serde(default = "default_false")]
    pub multi_pass_specialized: bool,

    #[serde(default, flatten)]
    pub agent: AgentConfig,

    #[serde(default, flatten)]
    pub verification: VerificationConfig,

    /// Enable enhanced feedback loop with per-category/file-pattern tracking
    /// and feedback-adjusted confidence scores (default false).
    #[serde(default)]
    pub enhanced_feedback: bool,

    /// Minimum number of feedback observations before adjusting confidence (default 5).
    #[serde(default = "default_feedback_min_observations")]
    pub feedback_min_observations: usize,

    /// Enable semantic repository retrieval for related code context.
    #[serde(default = "default_false")]
    pub semantic_rag: bool,

    #[serde(default = "default_semantic_rag_max_files")]
    pub semantic_rag_max_files: usize,

    #[serde(default = "default_semantic_rag_top_k")]
    pub semantic_rag_top_k: usize,

    #[serde(default = "default_semantic_rag_min_similarity")]
    pub semantic_rag_min_similarity: f32,

    /// Enable embedding-backed feedback memory on top of aggregate stats.
    #[serde(default)]
    pub semantic_feedback: bool,

    #[serde(default = "default_semantic_feedback_similarity")]
    pub semantic_feedback_similarity: f32,

    #[serde(default = "default_semantic_feedback_min_examples")]
    pub semantic_feedback_min_examples: usize,

    #[serde(default = "default_semantic_feedback_max_neighbors")]
    pub semantic_feedback_max_neighbors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PathConfig {
    #[serde(default)]
    pub focus: Vec<String>,

    #[serde(default)]
    pub ignore_patterns: Vec<String>,

    #[serde(default)]
    pub extra_context: Vec<String>,

    pub system_prompt: Option<String>,

    pub review_instructions: Option<String>,

    #[serde(default)]
    pub severity_overrides: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomContextConfig {
    #[serde(default)]
    pub scope: Option<String>,

    #[serde(default)]
    pub notes: Vec<String>,

    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PatternRepositoryConfig {
    pub source: String,

    #[serde(default)]
    pub scope: Option<String>,

    #[serde(default)]
    pub include_patterns: Vec<String>,

    #[serde(default = "default_pattern_repo_max_files")]
    pub max_files: usize,

    #[serde(default = "default_pattern_repo_max_lines")]
    pub max_lines: usize,

    #[serde(default)]
    pub rule_patterns: Vec<String>,

    #[serde(default = "default_pattern_repo_max_rules")]
    pub max_rules: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    #[serde(default = "default_true")]
    pub eslint: bool,

    #[serde(default = "default_true")]
    pub semgrep: bool,

    #[serde(default = "default_true")]
    pub duplicate_filter: bool,

    /// Regex-based secret detection on diff added lines (gitleaks-style patterns).
    #[serde(default = "default_true")]
    pub secret_scanner: bool,

    /// Supply-chain risk analysis for dependency manifest changes.
    #[serde(default = "default_true")]
    pub supply_chain: bool,

    /// Rust compile-regression analysis for high-confidence struct initializer removals.
    #[serde(default = "default_true")]
    pub rust_compile: bool,

    /// SARIF/code-scanning report paths to ingest as analyzer evidence.
    #[serde(default)]
    pub sarif_reports: Vec<String>,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            eslint: true,
            semgrep: true,
            duplicate_filter: true,
            secret_scanner: true,
            supply_chain: true,
            rust_compile: true,
            sarif_reports: Vec::new(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: default_model(),
            model_weak: default_model_weak(),
            model_fast: default_model_fast(),
            model_reasoning: default_model_reasoning(),
            model_embedding: None,
            generation_model_role: default_generation_model_role(),
            auditing_model_role: default_auditing_model_role(),
            fallback_models: Vec::new(),
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            max_context_chars: default_max_context_chars(),
            max_diff_chars: default_max_diff_chars(),
            context_max_chunks: default_context_max_chunks(),
            context_budget_chars: default_context_budget_chars(),
            min_confidence: default_min_confidence(),
            strictness: default_strictness(),
            comment_types: default_comment_types(),
            review_profile: None,
            review_instructions: None,
            review_rules_prose: None,
            smart_review_summary: true,
            smart_review_diagram: false,
            symbol_index: true,
            symbol_index_provider: default_symbol_index_provider(),
            symbol_index_max_files: default_symbol_index_max_files(),
            symbol_index_max_bytes: default_symbol_index_max_bytes(),
            symbol_index_max_locations: default_symbol_index_max_locations(),
            symbol_index_graph_hops: default_symbol_index_graph_hops(),
            symbol_index_graph_max_files: default_symbol_index_graph_max_files(),
            symbol_index_lsp_command: None,
            symbol_index_lsp_languages: default_symbol_index_lsp_languages(),
            triage_skip_deletion_only: false,
            feedback_path: default_feedback_path(),
            eval_trend_path: default_eval_trend_path(),
            feedback_eval_trend_path: default_feedback_eval_trend_path(),
            retention: RetentionConfig::default(),
            convention_store_path: None,
            system_prompt: None,
            api_key: None,
            base_url: None,
            adapter: None,
            context_window: None,
            openai_use_responses: None,
            adapter_timeout_secs: None,
            adapter_max_retries: None,
            adapter_retry_delay_ms: None,
            file_change_limit: None,
            auto_detect_instructions: true,
            output_language: None,
            include_fix_suggestions: true,
            feedback_suppression_threshold: default_feedback_suppression_threshold(),
            feedback_suppression_margin: default_feedback_suppression_margin(),
            vault: VaultConfig::default(),
            plugins: PluginConfig::default(),
            exclude_patterns: default_exclude_patterns(),
            paths: HashMap::new(),
            custom_context: Vec::new(),
            linked_issue_contexts: Vec::new(),
            document_contexts: Vec::new(),
            pattern_repositories: Vec::new(),
            rules_files: Vec::new(),
            max_active_rules: default_max_active_rules(),
            rule_priority: Vec::new(),
            providers: HashMap::new(),
            github: GitHubConfig::default(),
            jira: JiraConfig::default(),
            linear: LinearConfig::default(),
            automation: AutomationConfig::default(),
            server_security: ServerSecurityConfig::default(),
            multi_pass_specialized: false,
            agent: AgentConfig::default(),
            verification: VerificationConfig::default(),
            enhanced_feedback: false,
            feedback_min_observations: default_feedback_min_observations(),
            semantic_rag: false,
            semantic_rag_max_files: default_semantic_rag_max_files(),
            semantic_rag_top_k: default_semantic_rag_top_k(),
            semantic_rag_min_similarity: default_semantic_rag_min_similarity(),
            semantic_feedback: false,
            semantic_feedback_similarity: default_semantic_feedback_similarity(),
            semantic_feedback_min_examples: default_semantic_feedback_min_examples(),
            semantic_feedback_max_neighbors: default_semantic_feedback_max_neighbors(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        // Try to load from .diffscope.yml in current directory
        let config_path = PathBuf::from(".diffscope.yml");
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let config: Config = serde_yaml::from_str(&content)?;
            return Ok(config);
        }

        // Try alternative names
        let alt_config_path = PathBuf::from(".diffscope.yaml");
        if alt_config_path.exists() {
            let content = std::fs::read_to_string(&alt_config_path)?;
            let config: Config = serde_yaml::from_str(&content)?;
            return Ok(config);
        }

        // Try in home directory
        if let Some(home_dir) = dirs::home_dir() {
            let home_config = home_dir.join(".diffscope.yml");
            if home_config.exists() {
                let content = std::fs::read_to_string(&home_config)?;
                let config: Config = serde_yaml::from_str(&content)?;
                return Ok(config);
            }
        }

        // Return default config if no file found
        Ok(Config::default())
    }
}

/// CLI overrides collected from command-line arguments.
#[derive(Debug, Default)]
pub struct CliOverrides {
    pub temperature: Option<f32>,
    pub max_tokens: Option<usize>,
    pub strictness: Option<u8>,
    pub comment_types: Option<Vec<String>>,
    pub openai_responses: Option<bool>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub adapter: Option<String>,
    pub lsp_command: Option<String>,
    pub timeout: Option<u64>,
    pub max_retries: Option<usize>,
    pub file_change_limit: Option<usize>,
    pub output_language: Option<String>,
    pub vault_addr: Option<String>,
    pub vault_path: Option<String>,
    pub vault_key: Option<String>,
    pub agent_review: bool,
    pub agent_max_iterations: Option<usize>,
    pub agent_max_total_tokens: Option<usize>,
    pub verification_pass: Option<bool>,
}

impl Config {
    pub fn merge_with_cli(&mut self, cli_model: Option<String>, cli_prompt: Option<String>) {
        if let Some(model) = cli_model {
            self.model = model;
        }
        if let Some(prompt) = cli_prompt {
            self.system_prompt = Some(prompt);
        }
    }

    /// Apply CLI overrides to config. Only overrides fields that are Some/provided.
    pub fn apply_cli_overrides(&mut self, cli: CliOverrides) {
        if let Some(v) = cli.temperature {
            self.temperature = v;
        }
        if let Some(v) = cli.max_tokens {
            self.max_tokens = v;
        }
        if let Some(v) = cli.strictness {
            self.strictness = v;
        }
        if let Some(v) = cli.comment_types {
            self.comment_types = v;
        }
        if let Some(v) = cli.openai_responses {
            self.openai_use_responses = Some(v);
        }
        if let Some(v) = cli.base_url {
            self.base_url = Some(v);
        }
        if let Some(v) = cli.api_key {
            self.api_key = Some(v);
        }
        if let Some(v) = cli.adapter {
            self.adapter = Some(v);
        }
        if let Some(command) = cli.lsp_command {
            self.symbol_index = true;
            self.symbol_index_provider = "lsp".to_string();
            self.symbol_index_lsp_command = Some(command);
        }
        if let Some(v) = cli.timeout {
            self.adapter_timeout_secs = Some(v);
        }
        if let Some(v) = cli.max_retries {
            self.adapter_max_retries = Some(v);
        }
        if let Some(v) = cli.file_change_limit {
            self.file_change_limit = Some(v);
        }
        if let Some(v) = cli.output_language {
            self.output_language = Some(v);
        }
        if let Some(v) = cli.vault_addr {
            self.vault.addr = Some(v);
        }
        if let Some(v) = cli.vault_path {
            self.vault.path = Some(v);
        }
        if let Some(v) = cli.vault_key {
            self.vault.key = Some(v);
        }
        if cli.agent_review {
            self.agent.enabled = true;
        }
        if let Some(v) = cli.agent_max_iterations {
            self.agent.max_iterations = v;
        }
        if let Some(v) = cli.agent_max_total_tokens {
            self.agent.max_total_tokens = Some(v);
        }
        if let Some(v) = cli.verification_pass {
            self.verification.enabled = v;
        }
    }

    pub fn normalize(&mut self) {
        // Env var fallbacks for base_url and api_key
        if self.base_url.is_none() {
            self.base_url = std::env::var("DIFFSCOPE_BASE_URL")
                .ok()
                .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
                .filter(|s| !s.trim().is_empty());
        }
        if self.api_key.is_none() {
            self.api_key = std::env::var("DIFFSCOPE_API_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty());
        }

        apply_provider_env_api_key_fallback(&mut self.providers, "openai", "OPENAI_API_KEY");
        apply_provider_env_api_key_fallback(
            &mut self.providers,
            "openrouter",
            "OPENROUTER_API_KEY",
        );
        apply_provider_env_api_key_fallback(&mut self.providers, "anthropic", "ANTHROPIC_API_KEY");

        // Env var fallbacks for GitHub integration
        if self.github.token.is_none() {
            self.github.token = std::env::var("GITHUB_TOKEN")
                .ok()
                .filter(|s| !s.trim().is_empty());
        }
        if self.github.app_id.is_none() {
            self.github.app_id = std::env::var("DIFFSCOPE_GITHUB_APP_ID")
                .ok()
                .and_then(|raw| raw.trim().parse::<u64>().ok())
                .filter(|value| *value > 0);
        }
        if self.github.client_id.is_none() {
            self.github.client_id = std::env::var("DIFFSCOPE_GITHUB_CLIENT_ID")
                .ok()
                .or_else(|| std::env::var("GITHUB_CLIENT_ID").ok())
                .filter(|s| !s.trim().is_empty());
        }
        if self.github.client_secret.is_none() {
            self.github.client_secret = std::env::var("DIFFSCOPE_GITHUB_CLIENT_SECRET")
                .ok()
                .or_else(|| std::env::var("GITHUB_CLIENT_SECRET").ok())
                .filter(|s| !s.trim().is_empty());
        }
        if self.github.private_key.is_none() {
            self.github.private_key = std::env::var("DIFFSCOPE_GITHUB_PRIVATE_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty());
        }
        if self.github.webhook_secret.is_none() {
            self.github.webhook_secret = std::env::var("DIFFSCOPE_WEBHOOK_SECRET")
                .ok()
                .filter(|s| !s.trim().is_empty());
        }
        if let Ok(events) = std::env::var("DIFFSCOPE_GITHUB_AUTO_REVIEW_EVENTS") {
            self.github.auto_review_events = split_csv_list(&events);
        }
        if self.github.review_request_reviewers.is_empty() {
            self.github.review_request_reviewers =
                std::env::var("DIFFSCOPE_GITHUB_REVIEW_REQUEST_REVIEWERS")
                    .ok()
                    .map(|reviewers| split_csv_list(&reviewers))
                    .unwrap_or_default();
        }
        if self.jira.base_url.is_none() {
            self.jira.base_url = std::env::var("DIFFSCOPE_JIRA_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty());
        }
        if self.jira.email.is_none() {
            self.jira.email = std::env::var("DIFFSCOPE_JIRA_EMAIL")
                .ok()
                .filter(|s| !s.trim().is_empty());
        }
        if self.jira.api_token.is_none() {
            self.jira.api_token = std::env::var("DIFFSCOPE_JIRA_API_TOKEN")
                .ok()
                .filter(|s| !s.trim().is_empty());
        }
        if self.linear.api_key.is_none() {
            self.linear.api_key = std::env::var("DIFFSCOPE_LINEAR_API_KEY")
                .ok()
                .or_else(|| std::env::var("LINEAR_API_KEY").ok())
                .filter(|s| !s.trim().is_empty());
        }
        if self.automation.webhook_url.is_none() {
            self.automation.webhook_url = std::env::var("DIFFSCOPE_AUTOMATION_WEBHOOK_URL")
                .ok()
                .filter(|s| !s.trim().is_empty());
        }
        if self.automation.webhook_secret.is_none() {
            self.automation.webhook_secret = std::env::var("DIFFSCOPE_AUTOMATION_WEBHOOK_SECRET")
                .ok()
                .filter(|s| !s.trim().is_empty());
        }
        if self.server_security.api_key.is_none() {
            self.server_security.api_key = std::env::var("DIFFSCOPE_SERVER_API_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty());
        }
        if self.server_security.rate_limit_per_minute.is_none() {
            self.server_security.rate_limit_per_minute =
                std::env::var("DIFFSCOPE_SERVER_RATE_LIMIT_PER_MINUTE")
                    .ok()
                    .and_then(|raw| raw.trim().parse::<u32>().ok())
                    .filter(|value| *value > 0);
        }
        if self.server_security.api_key.is_some()
            && self.server_security.rate_limit_per_minute.unwrap_or(0) == 0
        {
            self.server_security.rate_limit_per_minute = Some(DEFAULT_SERVER_RATE_LIMIT_PER_MINUTE);
        }

        normalize_optional_trimmed(&mut self.api_key);
        normalize_optional_trimmed(&mut self.base_url);
        normalize_optional_trimmed(&mut self.github.token);
        normalize_optional_trimmed(&mut self.github.client_id);
        normalize_optional_trimmed(&mut self.github.client_secret);
        normalize_optional_trimmed(&mut self.github.private_key);
        normalize_optional_trimmed(&mut self.github.webhook_secret);
        normalize_lowercase_list(&mut self.github.auto_review_events);
        normalize_string_list(&mut self.github.review_request_reviewers);
        normalize_optional_trimmed(&mut self.jira.base_url);
        normalize_optional_trimmed(&mut self.jira.email);
        normalize_optional_trimmed(&mut self.jira.api_token);
        normalize_optional_trimmed(&mut self.linear.api_key);
        normalize_optional_trimmed(&mut self.automation.webhook_url);
        normalize_optional_trimmed(&mut self.automation.webhook_secret);
        normalize_optional_trimmed(&mut self.server_security.api_key);
        normalize_optional_trimmed(&mut self.vault.addr);
        normalize_optional_trimmed(&mut self.vault.token);
        normalize_optional_trimmed(&mut self.vault.path);
        normalize_optional_trimmed(&mut self.vault.key);
        normalize_optional_trimmed(&mut self.vault.mount);
        normalize_optional_trimmed(&mut self.vault.namespace);

        let mut normalized_providers = HashMap::new();
        for (name, mut provider) in std::mem::take(&mut self.providers) {
            let normalized_name = name.trim().to_ascii_lowercase();
            if normalized_name.is_empty() {
                continue;
            }
            normalize_optional_trimmed(&mut provider.api_key);
            normalize_optional_trimmed(&mut provider.base_url);
            validate_optional_http_url(
                &mut provider.base_url,
                &format!("providers.{normalized_name}.base_url"),
            );
            normalized_providers.insert(normalized_name, provider);
        }
        self.providers = normalized_providers;

        validate_optional_http_url(&mut self.base_url, "base_url");
        validate_optional_http_url(&mut self.automation.webhook_url, "automation_webhook_url");
        validate_optional_http_url(&mut self.jira.base_url, "jira_base_url");

        // Normalize adapter field
        if let Some(ref adapter) = self.adapter {
            let normalized = adapter.trim().to_lowercase();
            self.adapter = if matches!(
                normalized.as_str(),
                "openai" | "anthropic" | "openrouter" | "ollama"
            ) {
                Some(normalized)
            } else {
                None
            };
        }

        if self.model.trim().is_empty() {
            self.model = default_model();
        }
        normalize_optional_model_name(&mut self.model_weak);
        normalize_optional_model_name(&mut self.model_fast);
        normalize_optional_model_name(&mut self.model_reasoning);
        normalize_optional_model_name(&mut self.model_embedding);
        normalize_text_generation_role(
            &mut self.generation_model_role,
            default_generation_model_role(),
            "generation_model_role",
        );
        normalize_text_generation_role(
            &mut self.auditing_model_role,
            default_auditing_model_role(),
            "auditing_model_role",
        );
        normalize_text_generation_role(
            &mut self.verification.model_role,
            default_verification_model_role(),
            "verification_model_role",
        );
        self.verification.additional_model_roles = normalize_text_generation_roles(
            &self.verification.additional_model_roles,
            Some(self.verification.model_role),
        );

        if !self.temperature.is_finite() || self.temperature < 0.0 || self.temperature > 2.0 {
            warn!(
                "temperature {} is outside valid range 0.0..=2.0, resetting to default {}",
                self.temperature,
                default_temperature()
            );
            self.temperature = default_temperature();
        }

        if self.max_tokens == 0 {
            warn!(
                "max_tokens is 0, resetting to default {}",
                default_max_tokens()
            );
            self.max_tokens = default_max_tokens();
        } else if self.max_tokens > 128_000 {
            warn!(
                "max_tokens {} exceeds maximum 128000, clamping to 128000",
                self.max_tokens
            );
            self.max_tokens = 128_000;
        }
        if self.context_max_chunks == 0 {
            self.context_max_chunks = default_context_max_chunks();
        }
        if self.context_budget_chars == 0 {
            self.context_budget_chars = default_context_budget_chars();
        }

        if self.symbol_index_max_files == 0 {
            self.symbol_index_max_files = default_symbol_index_max_files();
        }
        if self.symbol_index_max_bytes == 0 {
            self.symbol_index_max_bytes = default_symbol_index_max_bytes();
        }
        if self.symbol_index_max_locations == 0 {
            self.symbol_index_max_locations = default_symbol_index_max_locations();
        }
        if self.symbol_index_graph_hops == 0 {
            self.symbol_index_graph_hops = default_symbol_index_graph_hops();
        }
        if self.symbol_index_graph_max_files == 0 {
            self.symbol_index_graph_max_files = default_symbol_index_graph_max_files();
        }

        let provider = self.symbol_index_provider.trim().to_lowercase();
        if provider.is_empty() || !matches!(provider.as_str(), "regex" | "lsp") {
            self.symbol_index_provider = default_symbol_index_provider();
        } else {
            self.symbol_index_provider = provider;
        }
        if self.feedback_path.as_os_str().is_empty() {
            self.feedback_path = default_feedback_path();
        }
        if self.eval_trend_path.as_os_str().is_empty() {
            self.eval_trend_path = default_eval_trend_path();
        }
        if self.feedback_eval_trend_path.as_os_str().is_empty() {
            self.feedback_eval_trend_path = default_feedback_eval_trend_path();
        }
        self.retention.normalize();

        if let Some(command) = &self.symbol_index_lsp_command {
            if command.trim().is_empty() {
                self.symbol_index_lsp_command = None;
            }
        }

        if self.symbol_index_provider == "lsp" && self.symbol_index_lsp_languages.is_empty() {
            self.symbol_index_lsp_languages = default_symbol_index_lsp_languages();
        }

        if !self.min_confidence.is_finite() {
            self.min_confidence = default_min_confidence();
        } else if !(0.0..=1.0).contains(&self.min_confidence) {
            self.min_confidence = self.min_confidence.clamp(0.0, 1.0);
        }
        if self.strictness == 0 {
            warn!(
                "strictness 0 is invalid (valid range: 1-3), resetting to default {}",
                default_strictness()
            );
            self.strictness = default_strictness();
        } else if self.strictness > 3 {
            warn!(
                "strictness {} is invalid (valid range: 1-3), clamping to 3",
                self.strictness
            );
            self.strictness = 3;
        }

        self.comment_types = normalize_comment_types(&self.comment_types);

        if let Some(profile) = &self.review_profile {
            let normalized = profile.trim().to_lowercase();
            self.review_profile = if normalized.is_empty() {
                None
            } else if matches!(normalized.as_str(), "balanced" | "chill" | "assertive") {
                Some(normalized)
            } else {
                None
            };
        }

        if let Some(instructions) = &self.review_instructions {
            if instructions.trim().is_empty() {
                self.review_instructions = None;
            }
        }

        let mut normalized_custom_context = Vec::new();
        for mut entry in std::mem::take(&mut self.custom_context) {
            entry.scope = entry.scope.and_then(|scope| {
                let trimmed = scope.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            });

            entry.notes = entry
                .notes
                .into_iter()
                .map(|note| note.trim().to_string())
                .filter(|note| !note.is_empty())
                .collect();
            entry.files = entry
                .files
                .into_iter()
                .map(|file| file.trim().to_string())
                .filter(|file| !file.is_empty())
                .collect();

            if entry.notes.is_empty() && entry.files.is_empty() {
                continue;
            }
            normalized_custom_context.push(entry);
        }
        self.custom_context = normalized_custom_context;

        let mut normalized_pattern_repositories = Vec::new();
        for mut repo in std::mem::take(&mut self.pattern_repositories) {
            repo.source = repo.source.trim().to_string();
            if repo.source.is_empty() {
                continue;
            }
            repo.scope = repo.scope.and_then(|scope| {
                let trimmed = scope.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            });
            repo.include_patterns = repo
                .include_patterns
                .into_iter()
                .map(|pattern| pattern.trim().to_string())
                .filter(|pattern| !pattern.is_empty())
                .collect();
            if repo.include_patterns.is_empty() {
                repo.include_patterns.push("**/*".to_string());
            }
            if repo.max_files == 0 {
                repo.max_files = default_pattern_repo_max_files();
            }
            if repo.max_lines == 0 {
                repo.max_lines = default_pattern_repo_max_lines();
            }
            if repo.max_rules == 0 {
                repo.max_rules = default_pattern_repo_max_rules();
            }
            repo.rule_patterns = repo
                .rule_patterns
                .into_iter()
                .map(|pattern| pattern.trim().to_string())
                .filter(|pattern| !pattern.is_empty())
                .collect();

            normalized_pattern_repositories.push(repo);
        }
        self.pattern_repositories = normalized_pattern_repositories;

        self.rules_files = self
            .rules_files
            .iter()
            .map(|pattern| pattern.trim().to_string())
            .filter(|pattern| !pattern.is_empty())
            .collect();
        if self.max_active_rules == 0 {
            self.max_active_rules = default_max_active_rules();
        }
        self.rule_priority = self
            .rule_priority
            .iter()
            .map(|rule| rule.trim().to_ascii_lowercase())
            .filter(|rule| !rule.is_empty())
            .fold(Vec::new(), |mut acc, rule| {
                if !acc.contains(&rule) {
                    acc.push(rule);
                }
                acc
            });

        // Clamp adapter timeout to reasonable range (5s - 600s)
        if let Some(timeout) = self.adapter_timeout_secs {
            if timeout == 0 {
                self.adapter_timeout_secs = None; // use default
            } else {
                self.adapter_timeout_secs = Some(timeout.clamp(5, 600));
            }
        }
        // Clamp adapter retries (0-10)
        if let Some(retries) = self.adapter_max_retries {
            self.adapter_max_retries = Some(retries.min(10));
        }
        // Clamp retry delay (50ms - 30s)
        if let Some(delay) = self.adapter_retry_delay_ms {
            if delay == 0 {
                self.adapter_retry_delay_ms = None;
            } else {
                self.adapter_retry_delay_ms = Some(delay.clamp(50, 30_000));
            }
        }
        // Normalize output language
        if let Some(ref lang) = self.output_language {
            let trimmed = lang.trim().to_lowercase();
            self.output_language = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            };
        }
        // Ensure suppression thresholds are reasonable
        if self.feedback_suppression_threshold == 0 {
            self.feedback_suppression_threshold = default_feedback_suppression_threshold();
        }
        if self.semantic_rag_max_files == 0 {
            self.semantic_rag_max_files = default_semantic_rag_max_files();
        }
        if self.semantic_rag_top_k == 0 {
            self.semantic_rag_top_k = default_semantic_rag_top_k();
        }
        if !self.semantic_rag_min_similarity.is_finite() {
            self.semantic_rag_min_similarity = default_semantic_rag_min_similarity();
        } else {
            self.semantic_rag_min_similarity = self.semantic_rag_min_similarity.clamp(0.0, 1.0);
        }
        if !self.semantic_feedback_similarity.is_finite() {
            self.semantic_feedback_similarity = default_semantic_feedback_similarity();
        } else {
            self.semantic_feedback_similarity = self.semantic_feedback_similarity.clamp(0.0, 1.0);
        }
        if self.semantic_feedback_min_examples == 0 {
            self.semantic_feedback_min_examples = default_semantic_feedback_min_examples();
        }
        if self.semantic_feedback_max_neighbors == 0 {
            self.semantic_feedback_max_neighbors = default_semantic_feedback_max_neighbors();
        }
    }

    pub fn get_path_config(&self, file_path: &Path) -> Option<&PathConfig> {
        let file_path_str = file_path.to_string_lossy();

        // Find the most specific matching path
        let mut best_match: Option<(&String, &PathConfig)> = None;

        for (pattern, config) in &self.paths {
            if self.path_matches(&file_path_str, pattern) {
                // Keep the most specific match (longest pattern)
                if best_match
                    .as_ref()
                    .is_none_or(|(best_pattern, _)| pattern.len() > best_pattern.len())
                {
                    best_match = Some((pattern, config));
                }
            }
        }

        best_match.map(|(_, config)| config)
    }

    pub fn should_exclude(&self, file_path: &Path) -> bool {
        let file_path_str = file_path.to_string_lossy();

        // Check global exclude patterns
        for pattern in &self.exclude_patterns {
            if self.path_matches(&file_path_str, pattern) {
                return true;
            }
        }

        // Check path-specific ignore patterns
        if let Some(path_config) = self.get_path_config(file_path) {
            for pattern in &path_config.ignore_patterns {
                if self.path_matches(&file_path_str, pattern) {
                    return true;
                }
            }
        }

        false
    }

    pub fn matching_custom_context(&self, file_path: &Path) -> Vec<&CustomContextConfig> {
        let file_path_str = file_path.to_string_lossy();
        self.custom_context
            .iter()
            .filter(|entry| match entry.scope.as_deref() {
                Some(scope) => self.path_matches(&file_path_str, scope),
                None => true,
            })
            .collect()
    }

    pub fn effective_min_confidence(&self) -> f32 {
        let strictness_floor = match self.strictness {
            1 => 0.85,
            2 => 0.65,
            _ => 0.45,
        };
        self.min_confidence.max(strictness_floor).clamp(0.0, 1.0)
    }

    pub fn matching_pattern_repositories(&self, file_path: &Path) -> Vec<&PatternRepositoryConfig> {
        let file_path_str = file_path.to_string_lossy();
        self.pattern_repositories
            .iter()
            .filter(|repo| match repo.scope.as_deref() {
                Some(scope) => self.path_matches(&file_path_str, scope),
                None => true,
            })
            .collect()
    }

    /// Build a ModelConfig from this Config.
    pub fn to_model_config(&self) -> crate::adapters::llm::ModelConfig {
        crate::adapters::llm::ModelConfig {
            model_name: self.model.clone(),
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            temperature: self.temperature,
            max_tokens: self.max_tokens,
            openai_use_responses: self.openai_use_responses,
            adapter_override: self.adapter.clone(),
            timeout_secs: self.adapter_timeout_secs,
            max_retries: self.adapter_max_retries,
            retry_delay_ms: self.adapter_retry_delay_ms,
        }
    }

    /// Get the model name for a specific role, falling back to the primary model.
    pub fn model_for_role(&self, role: ModelRole) -> &str {
        match role {
            ModelRole::Primary => &self.model,
            ModelRole::Weak => self.model_weak.as_deref().unwrap_or(&self.model),
            ModelRole::Reasoning => self.model_reasoning.as_deref().unwrap_or(&self.model),
            ModelRole::Embedding => self.model_embedding.as_deref().unwrap_or(&self.model),
            ModelRole::Fast => self
                .model_fast
                .as_deref()
                .or(self.model_weak.as_deref())
                .unwrap_or(&self.model),
        }
    }

    pub fn generation_model_name(&self) -> &str {
        self.model_for_role(self.generation_model_role)
    }

    pub fn auditing_model_name(&self) -> &str {
        self.model_for_role(self.auditing_model_role)
    }

    pub fn resolved_provider_for_role(&self, role: ModelRole) -> ResolvedProviderConfig {
        self.resolved_provider_for_model(self.model_for_role(role))
    }

    pub fn resolved_provider_for_model(&self, model_name: &str) -> ResolvedProviderConfig {
        let provider = infer_provider_label(
            model_name,
            self.adapter.as_deref(),
            self.base_url.as_deref(),
        );
        let provider_config = provider
            .as_deref()
            .and_then(|name| self.providers.get(name))
            .filter(|provider| provider.enabled);

        ResolvedProviderConfig {
            provider,
            api_key: provider_config
                .and_then(|provider| provider.api_key.clone())
                .or_else(|| self.api_key.clone()),
            base_url: provider_config
                .and_then(|provider| provider.base_url.clone())
                .or_else(|| self.base_url.clone()),
            adapter: self.adapter.clone(),
        }
    }

    /// Build a ModelConfig for a specific role.
    pub fn to_model_config_for_role(&self, role: ModelRole) -> crate::adapters::llm::ModelConfig {
        let resolved_provider = self.resolved_provider_for_role(role);
        crate::adapters::llm::ModelConfig {
            model_name: self.model_for_role(role).to_string(),
            api_key: resolved_provider.api_key,
            base_url: resolved_provider.base_url,
            temperature: self.temperature,
            max_tokens: self.max_tokens,
            openai_use_responses: self.openai_use_responses,
            adapter_override: resolved_provider.adapter,
            timeout_secs: self.adapter_timeout_secs,
            max_retries: self.adapter_max_retries,
            retry_delay_ms: self.adapter_retry_delay_ms,
        }
    }

    pub fn set_model_for_role(&mut self, role: ModelRole, model_name: impl Into<String>) {
        let model_name = model_name.into().trim().to_string();
        if model_name.is_empty() {
            return;
        }

        match role {
            ModelRole::Primary => self.model = model_name,
            ModelRole::Weak => self.model_weak = Some(model_name),
            ModelRole::Reasoning => self.model_reasoning = Some(model_name),
            ModelRole::Embedding => self.model_embedding = Some(model_name),
            ModelRole::Fast => self.model_fast = Some(model_name),
        }
    }

    pub fn inferred_provider_label_for_role(&self, role: ModelRole) -> Option<String> {
        self.resolved_provider_for_role(role).provider
    }

    pub fn validation_issues(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();

        for provider_name in self.providers.keys() {
            if !matches!(
                provider_name.as_str(),
                "openai" | "openrouter" | "anthropic" | "ollama"
            ) {
                issues.push(ConfigValidationIssue::warning(
                    format!("providers.{provider_name}"),
                    format!(
                        "Provider '{}' is not one of the built-in DiffScope providers (openai, openrouter, anthropic, ollama) and will only be used if a matching adapter is selected.",
                        provider_name
                    ),
                ));
            }
        }

        let selected_roles = [
            ModelRole::Primary,
            ModelRole::Weak,
            ModelRole::Reasoning,
            ModelRole::Embedding,
            ModelRole::Fast,
        ]
        .into_iter()
        .map(|role| (role, self.resolved_provider_for_role(role)))
        .collect::<Vec<_>>();

        let unique_selected_providers = selected_roles
            .iter()
            .filter_map(|(_, provider)| provider.provider.clone())
            .collect::<std::collections::BTreeSet<_>>();

        if unique_selected_providers.len() > 1
            && (self.api_key.is_some() || self.base_url.is_some() || self.adapter.is_some())
        {
            issues.push(ConfigValidationIssue::warning(
                "providers",
                "Multiple model roles resolve to different providers, but legacy top-level api_key/base_url/adapter settings are still configured. Prefer providers.<name> entries so one provider fallback does not leak into every role.",
            ));
        }

        if unique_selected_providers.len() > 1
            && vault_is_partially_or_fully_configured(&self.vault)
        {
            issues.push(ConfigValidationIssue::warning(
                "vault",
                "Vault currently resolves only the legacy api_key field. Multi-provider installs should configure provider-specific secrets via providers.<name>.api_key or provider-specific environment variables.",
            ));
        }

        for (role, provider) in &selected_roles {
            let Some(provider_name) = provider.provider.as_deref() else {
                continue;
            };
            if matches!(provider_name, "ollama") || is_local_base_url(provider.base_url.as_deref())
            {
                continue;
            }
            if provider.api_key.is_none() {
                issues.push(ConfigValidationIssue::warning(
                    format!("providers.{provider_name}.api_key"),
                    format!(
                        "Model role '{}' resolves to provider '{}' but no API key is configured through providers.{provider_name}.api_key, provider-specific environment variables, or the legacy api_key fallback.",
                        role.as_str(),
                        provider_name,
                    ),
                ));
            }
            if self
                .providers
                .get(provider_name)
                .is_some_and(|provider_config| !provider_config.enabled)
            {
                issues.push(ConfigValidationIssue::warning(
                    format!("providers.{provider_name}.enabled"),
                    format!(
                        "Model role '{}' resolves to disabled provider '{}'; DiffScope will fall back to legacy top-level provider settings.",
                        role.as_str(),
                        provider_name,
                    ),
                ));
            }
        }

        let github_app_partial = self.github.app_id.is_some() ^ self.github.private_key.is_some();
        if github_app_partial {
            issues.push(ConfigValidationIssue::error(
                "github_app",
                "GitHub App authentication requires both github_app_id and github_private_key when either field is configured.",
            ));
        }
        if self.github.client_secret.is_some() && self.github.client_id.is_none() {
            issues.push(ConfigValidationIssue::warning(
                "github_client_secret",
                "github_client_secret is set without github_client_id. The device flow only uses github_client_id today.",
            ));
        }
        for event in &self.github.auto_review_events {
            if !matches!(
                event.as_str(),
                "opened" | "synchronize" | "reopened" | "review_requested"
            ) {
                issues.push(ConfigValidationIssue::warning(
                    "github_auto_review_events",
                    format!(
                        "Unsupported GitHub pull_request action '{}'; supported values are opened, synchronize, reopened, and review_requested.",
                        event
                    ),
                ));
            }
        }
        if self
            .github
            .auto_review_events
            .iter()
            .any(|event| event == "review_requested")
            && self.github.review_request_reviewers.is_empty()
        {
            issues.push(ConfigValidationIssue::warning(
                "github_review_request_reviewers",
                "review_requested automation is enabled but no requested reviewer logins are configured.",
            ));
        }

        let jira_fields = [
            self.jira.base_url.as_ref(),
            self.jira.email.as_ref(),
            self.jira.api_token.as_ref(),
        ];
        let jira_present = jira_fields.iter().filter(|value| value.is_some()).count();
        if jira_present > 0 && jira_present < jira_fields.len() {
            issues.push(ConfigValidationIssue::error(
                "jira",
                "Jira integration requires jira_base_url, jira_email, and jira_api_token together.",
            ));
        }

        if vault_fields_partially_configured(&self.vault) {
            issues.push(ConfigValidationIssue::error(
                "vault",
                "Vault configuration is incomplete. Set vault_addr, vault_path, and vault_token together (vault_key remains optional).",
            ));
        }

        issues
    }

    /// Resolve which provider to use based on configuration.
    ///
    /// Returns `(api_key, base_url, adapter)` by checking:
    /// 1. If `adapter` is explicitly set and a matching enabled provider exists, use it.
    /// 2. If no adapter is set, infer from the model name.
    /// 3. Fall back to top-level `api_key`/`base_url`.
    #[allow(dead_code)]
    pub fn resolve_provider(&self) -> (Option<String>, Option<String>, Option<String>) {
        let resolved_provider = self.resolved_provider_for_role(ModelRole::Primary);
        (
            resolved_provider.api_key,
            resolved_provider.base_url,
            resolved_provider.adapter,
        )
    }

    /// Try to resolve the API key from Vault if Vault is configured and api_key is not set.
    pub async fn resolve_vault_api_key(&mut self) -> Result<()> {
        if self.api_key.is_some() {
            return Ok(());
        }

        let vault_config = crate::vault::try_build_vault_config(
            self.vault.addr.as_deref(),
            self.vault.token.as_deref(),
            self.vault.path.as_deref(),
            self.vault.key.as_deref(),
            self.vault.mount.as_deref(),
            self.vault.namespace.as_deref(),
        );

        if let Some(vc) = vault_config {
            tracing::info!("Fetching API key from Vault at {}", vc.addr);
            let secret = crate::vault::fetch_secret(&vc).await?;
            self.api_key = Some(secret);
            tracing::info!("API key loaded from Vault");
        }

        Ok(())
    }

    /// Returns true if the configured base_url points to a local/self-hosted server.
    pub fn is_local_endpoint(&self) -> bool {
        match self.base_url.as_deref() {
            Some(url) => crate::adapters::common::is_local_endpoint(url),
            None => false,
        }
    }

    fn path_matches(&self, path: &str, pattern: &str) -> bool {
        // Simple glob matching
        if pattern.contains('*') {
            if let Ok(glob_pattern) = glob::Pattern::new(pattern) {
                glob_pattern.matches(path)
            } else {
                false
            }
        } else {
            // Path prefix matching with component boundary check
            path == pattern || path.starts_with(&format!("{}/", pattern.trim_end_matches('/')))
        }
    }
}

fn validate_optional_http_url(url: &mut Option<String>, field_name: &str) {
    let Some(raw_url) = url.clone() else {
        return;
    };

    match url::Url::parse(&raw_url) {
        Ok(parsed) => {
            if !matches!(parsed.scheme(), "http" | "https") {
                warn!(
                    "{} '{}' uses unsupported scheme '{}' (expected http or https), ignoring",
                    field_name,
                    raw_url,
                    parsed.scheme()
                );
                *url = None;
            } else if parsed.host().is_none() {
                warn!("{} '{}' has no valid host, ignoring", field_name, raw_url);
                *url = None;
            }
        }
        Err(err) => {
            warn!(
                "{} '{}' is not a valid URL ({}), ignoring",
                field_name, raw_url, err
            );
            *url = None;
        }
    }
}

fn normalize_optional_trimmed(value: &mut Option<String>) {
    *value = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
}

fn split_csv_list(value: &str) -> Vec<String> {
    value.split(',').map(ToOwned::to_owned).collect()
}

fn normalize_string_list(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain_mut(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return false;
        }

        let normalized_key = trimmed.to_ascii_lowercase();
        if !seen.insert(normalized_key) {
            return false;
        }

        if trimmed.len() != value.len() {
            *value = trimmed.to_string();
        }
        true
    });
}

fn normalize_lowercase_list(values: &mut Vec<String>) {
    normalize_string_list(values);
    for value in values {
        *value = value.to_ascii_lowercase();
    }
}

fn apply_provider_env_api_key_fallback(
    providers: &mut HashMap<String, ProviderConfig>,
    provider_name: &str,
    env_name: &str,
) {
    let env_value = std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if env_value.is_none() && !providers.contains_key(provider_name) {
        return;
    }

    let provider = providers.entry(provider_name.to_string()).or_default();
    normalize_optional_trimmed(&mut provider.api_key);
    if provider.api_key.is_none() {
        provider.api_key = env_value;
    }
}

fn vault_is_partially_or_fully_configured(vault: &VaultConfig) -> bool {
    vault.addr.is_some()
        || vault.token.is_some()
        || vault.path.is_some()
        || vault.key.is_some()
        || vault.mount.is_some()
        || vault.namespace.is_some()
        || std::env::var("VAULT_ADDR")
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
        || std::env::var("VAULT_PATH")
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
        || std::env::var("VAULT_TOKEN")
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
}

fn vault_fields_partially_configured(vault: &VaultConfig) -> bool {
    let configured = [
        vault.addr.as_ref(),
        vault.path.as_ref(),
        vault.token.as_ref(),
    ]
    .into_iter()
    .filter(|value| value.is_some())
    .count();
    configured > 0 && configured < 3
}

fn is_local_base_url(base_url: Option<&str>) -> bool {
    base_url
        .map(crate::adapters::common::is_local_endpoint)
        .unwrap_or(false)
}

fn default_model() -> String {
    "anthropic/claude-opus-4.5".to_string()
}

fn default_model_weak() -> Option<String> {
    Some("anthropic/claude-sonnet-4.5".to_string())
}

fn default_model_fast() -> Option<String> {
    Some("anthropic/claude-sonnet-4.5".to_string())
}

fn default_model_reasoning() -> Option<String> {
    Some("anthropic/claude-opus-4.5".to_string())
}

fn default_temperature() -> f32 {
    0.2
}

fn default_github_auto_review_events() -> Vec<String> {
    vec!["opened".to_string(), "synchronize".to_string()]
}

fn default_max_tokens() -> usize {
    4000
}

fn default_max_context_chars() -> usize {
    20000
}

fn default_max_diff_chars() -> usize {
    40000
}

fn default_exclude_patterns() -> Vec<String> {
    [
        "*.min.js",
        "*.min.css",
        "*.map",
        "*.generated.*",
        "*.pb.go",
        "*.pb.rs",
        "*_generated.go",
        "vendor/**",
        "node_modules/**",
        ".git/**",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn default_context_max_chunks() -> usize {
    24
}

fn default_context_budget_chars() -> usize {
    24000
}

fn default_min_confidence() -> f32 {
    0.0
}

fn default_strictness() -> u8 {
    2
}

fn default_comment_types() -> Vec<String> {
    vec![
        "logic".to_string(),
        "syntax".to_string(),
        "style".to_string(),
        "informational".to_string(),
    ]
}

fn default_symbol_index_max_files() -> usize {
    500
}

fn default_symbol_index_max_bytes() -> usize {
    200_000
}

fn default_symbol_index_max_locations() -> usize {
    5
}

fn default_symbol_index_graph_hops() -> usize {
    2
}

fn default_symbol_index_graph_max_files() -> usize {
    12
}

fn default_symbol_index_provider() -> String {
    "regex".to_string()
}

fn default_symbol_index_lsp_languages() -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("rs".to_string(), "rust".to_string());
    map
}

fn default_feedback_path() -> PathBuf {
    PathBuf::from(".diffscope.feedback.json")
}

fn default_review_retention_max_age_days() -> i64 {
    30
}

fn default_review_retention_max_count() -> usize {
    1000
}

fn default_eval_artifact_retention_max_age_days() -> i64 {
    30
}

fn default_trend_history_max_entries() -> usize {
    200
}

fn default_eval_trend_path() -> PathBuf {
    PathBuf::from(".diffscope.eval-trend.json")
}

fn default_feedback_eval_trend_path() -> PathBuf {
    PathBuf::from(".diffscope.feedback-eval-trend.json")
}

fn default_pattern_repo_max_files() -> usize {
    8
}

fn default_pattern_repo_max_lines() -> usize {
    200
}

fn default_pattern_repo_max_rules() -> usize {
    200
}

fn default_max_active_rules() -> usize {
    30
}

fn default_feedback_suppression_threshold() -> usize {
    3
}

fn default_feedback_suppression_margin() -> usize {
    2
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_generation_model_role() -> ModelRole {
    ModelRole::Primary
}

fn default_auditing_model_role() -> ModelRole {
    ModelRole::Reasoning
}

fn default_agent_max_iterations() -> usize {
    10
}

fn default_verification_model_role() -> ModelRole {
    ModelRole::Weak
}

fn default_verification_additional_model_roles() -> Vec<ModelRole> {
    vec![ModelRole::Reasoning]
}

fn default_verification_consensus_mode() -> VerificationConsensusMode {
    VerificationConsensusMode::Any
}

fn default_verification_min_score() -> u8 {
    5
}

fn default_verification_max_comments() -> usize {
    20
}

fn default_feedback_min_observations() -> usize {
    5
}

fn default_semantic_rag_max_files() -> usize {
    500
}

fn default_semantic_rag_top_k() -> usize {
    5
}

fn default_semantic_rag_min_similarity() -> f32 {
    0.25
}

fn default_semantic_feedback_similarity() -> f32 {
    0.82
}

fn default_semantic_feedback_min_examples() -> usize {
    3
}

fn default_semantic_feedback_max_neighbors() -> usize {
    8
}

fn normalize_optional_model_name(value: &mut Option<String>) {
    let Some(current) = value.as_ref() else {
        return;
    };

    let trimmed = current.trim();
    if trimmed.is_empty() {
        *value = None;
    } else if trimmed.len() != current.len() {
        *value = Some(trimmed.to_string());
    }
}

fn normalize_text_generation_role(role: &mut ModelRole, default_role: ModelRole, field_name: &str) {
    if role.supports_text_generation() {
        return;
    }

    warn!(
        "{} does not support text generation; resetting to {}",
        field_name,
        default_role.as_str()
    );
    *role = default_role;
}

fn normalize_text_generation_roles(
    roles: &[ModelRole],
    exclude_role: Option<ModelRole>,
) -> Vec<ModelRole> {
    let mut normalized = Vec::new();
    for role in roles.iter().copied() {
        if !role.supports_text_generation()
            || Some(role) == exclude_role
            || normalized.contains(&role)
        {
            continue;
        }
        normalized.push(role);
    }
    normalized
}

fn infer_provider_label(
    model_name: &str,
    adapter: Option<&str>,
    base_url: Option<&str>,
) -> Option<String> {
    if base_url.is_some_and(|value| value.contains("openrouter.ai")) {
        return Some("openrouter".to_string());
    }

    if let Some(adapter) = adapter.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(adapter.to_ascii_lowercase());
    }

    let normalized = model_name.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    if let Some((vendor, _)) = normalized.split_once('/') {
        return match vendor {
            "anthropic" => Some("anthropic".to_string()),
            _ => Some("openrouter".to_string()),
        };
    }

    if normalized.starts_with("claude") {
        Some("anthropic".to_string())
    } else if normalized.starts_with("openai/")
        || normalized.starts_with("gpt")
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
        || normalized.starts_with("o5")
    {
        Some("openai".to_string())
    } else if normalized.starts_with("ollama:") || is_local_base_url(base_url) {
        Some("ollama".to_string())
    } else {
        None
    }
}

fn normalize_comment_types(values: &[String]) -> Vec<String> {
    if values.is_empty() {
        return default_comment_types();
    }

    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim().to_lowercase();
        if !matches!(
            value.as_str(),
            "logic" | "syntax" | "style" | "informational"
        ) {
            continue;
        }
        if !normalized.contains(&value) {
            normalized.push(value);
        }
    }

    if normalized.is_empty() {
        default_comment_types()
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_clamps_values() {
        let mut config = Config {
            model: "   ".to_string(),
            temperature: 5.0,
            max_tokens: 0,
            min_confidence: 2.0,
            strictness: 0,
            review_profile: Some("ASSERTIVE".to_string()),
            ..Config::default()
        };

        config.normalize();

        assert_eq!(config.model, default_model());
        assert_eq!(config.temperature, default_temperature());
        assert_eq!(config.max_tokens, default_max_tokens());
        assert_eq!(config.min_confidence, 1.0);
        assert_eq!(config.strictness, default_strictness());
        assert_eq!(config.review_profile.as_deref(), Some("assertive"));
    }

    #[test]
    fn normalize_comment_types_filters_unknown_values() {
        let mut config = Config {
            comment_types: vec![
                " LOGIC ".to_string(),
                "style".to_string(),
                "unknown".to_string(),
                "STYLE".to_string(),
            ],
            ..Config::default()
        };

        config.normalize();

        assert_eq!(config.comment_types, vec!["logic", "style"]);
    }

    #[test]
    fn path_matches_respects_component_boundary() {
        let config = Config::default();

        // Exact prefix with separator should match
        assert!(config.path_matches("src/file.rs", "src/"));
        assert!(config.path_matches("src/sub/file.rs", "src"));

        // Glob patterns should still work
        assert!(config.path_matches("src/file.rs", "src/*.rs"));

        // Non-glob pattern must NOT match a different path component
        // "src" should not match "srcfoo/file.rs" or "src-backup/file.rs"
        assert!(
            !config.path_matches("srcfoo/file.rs", "src"),
            "pattern 'src' should not match 'srcfoo/file.rs' (different path component)"
        );
        assert!(
            !config.path_matches("src-backup/file.rs", "src"),
            "pattern 'src' should not match 'src-backup/file.rs'"
        );

        // Exact match should work
        assert!(config.path_matches("src/file.rs", "src/file.rs"));
    }

    #[test]
    fn normalize_validates_base_url_valid_http() {
        let mut config = Config {
            base_url: Some("http://localhost:11434".to_string()),
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.base_url.as_deref(), Some("http://localhost:11434"));
    }

    #[test]
    fn normalize_validates_base_url_valid_https() {
        let mut config = Config {
            base_url: Some("https://api.openai.com/v1".to_string()),
            ..Config::default()
        };
        config.normalize();
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
    }

    #[test]
    fn normalize_rejects_base_url_bad_scheme() {
        let mut config = Config {
            base_url: Some("ftp://example.com".to_string()),
            ..Config::default()
        };
        config.normalize();
        assert!(config.base_url.is_none());
    }

    #[test]
    fn normalize_rejects_base_url_no_host() {
        let mut config = Config {
            base_url: Some("http://".to_string()),
            ..Config::default()
        };
        config.normalize();
        assert!(config.base_url.is_none());
    }

    #[test]
    fn multi_pass_specialized_config_default_false() {
        let config = Config::default();
        assert!(!config.multi_pass_specialized);
    }

    #[test]
    fn normalize_rejects_base_url_not_a_url() {
        let mut config = Config {
            base_url: Some("not a url at all".to_string()),
            ..Config::default()
        };
        config.normalize();
        assert!(config.base_url.is_none());
    }

    #[test]
    fn normalize_rejects_base_url_javascript_scheme() {
        let mut config = Config {
            base_url: Some("javascript:alert(1)".to_string()),
            ..Config::default()
        };
        config.normalize();
        assert!(config.base_url.is_none());
    }

    #[test]
    fn normalize_accepts_automation_webhook_url_https() {
        let mut config = Config {
            automation: AutomationConfig {
                webhook_url: Some("https://automation.example.com/hooks/reviews".to_string()),
                ..AutomationConfig::default()
            },
            ..Config::default()
        };

        config.normalize();

        assert_eq!(
            config.automation.webhook_url.as_deref(),
            Some("https://automation.example.com/hooks/reviews")
        );
    }

    #[test]
    fn normalize_rejects_automation_webhook_url_bad_scheme() {
        let mut config = Config {
            automation: AutomationConfig {
                webhook_url: Some("ftp://automation.example.com/hooks/reviews".to_string()),
                ..AutomationConfig::default()
            },
            ..Config::default()
        };

        config.normalize();

        assert!(config.automation.webhook_url.is_none());
    }

    #[test]
    fn normalize_defaults_server_rate_limit_when_api_key_present() {
        let mut config = Config {
            server_security: ServerSecurityConfig {
                api_key: Some("shared-key".to_string()),
                rate_limit_per_minute: None,
            },
            ..Config::default()
        };

        config.normalize();

        assert_eq!(
            config.server_security.rate_limit_per_minute,
            Some(DEFAULT_SERVER_RATE_LIMIT_PER_MINUTE)
        );
    }

    #[test]
    fn normalize_preserves_explicit_server_rate_limit() {
        let mut config = Config {
            server_security: ServerSecurityConfig {
                api_key: Some("shared-key".to_string()),
                rate_limit_per_minute: Some(120),
            },
            ..Config::default()
        };

        config.normalize();

        assert_eq!(config.server_security.rate_limit_per_minute, Some(120));
    }

    #[test]
    fn normalize_clamps_max_tokens_above_limit() {
        let mut config = Config {
            max_tokens: 200_000,
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.max_tokens, 128_000);
    }

    #[test]
    fn normalize_accepts_max_tokens_at_limit() {
        let mut config = Config {
            max_tokens: 128_000,
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.max_tokens, 128_000);
    }

    #[test]
    fn normalize_strictness_warns_and_clamps_above_3() {
        let mut config = Config {
            strictness: 5,
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.strictness, 3);
    }

    #[test]
    fn normalize_strictness_warns_and_defaults_zero() {
        let mut config = Config {
            strictness: 0,
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.strictness, default_strictness());
    }

    #[test]
    fn normalize_accepts_valid_strictness() {
        for s in 1..=3 {
            let mut config = Config {
                strictness: s,
                ..Config::default()
            };
            config.normalize();
            assert_eq!(config.strictness, s);
        }
    }

    #[test]
    fn normalize_temperature_negative() {
        let mut config = Config {
            temperature: -0.5,
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.temperature, default_temperature());
    }

    #[test]
    fn normalize_temperature_nan() {
        let mut config = Config {
            temperature: f32::NAN,
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.temperature, default_temperature());
    }

    #[test]
    fn normalize_temperature_infinity() {
        let mut config = Config {
            temperature: f32::INFINITY,
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.temperature, default_temperature());
    }

    #[test]
    fn normalize_adapter_timeout_clamps_to_max() {
        let mut config = Config {
            adapter_timeout_secs: Some(9999),
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.adapter_timeout_secs, Some(600));
    }

    #[test]
    fn normalize_adapter_timeout_zero_clears() {
        let mut config = Config {
            adapter_timeout_secs: Some(0),
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.adapter_timeout_secs, None);
    }

    #[test]
    fn normalize_adapter_retries_clamps() {
        let mut config = Config {
            adapter_max_retries: Some(50),
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.adapter_max_retries, Some(10));
    }

    #[test]
    fn normalize_preserves_openrouter_adapter_override() {
        let mut config = Config {
            adapter: Some("OpenRouter".to_string()),
            ..Config::default()
        };

        config.normalize();

        assert_eq!(config.adapter.as_deref(), Some("openrouter"));
    }

    #[test]
    fn normalize_output_language_trims() {
        let mut config = Config {
            output_language: Some("  JA  ".to_string()),
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.output_language.as_deref(), Some("ja"));
    }

    #[test]
    fn normalize_output_language_empty_clears() {
        let mut config = Config {
            output_language: Some("   ".to_string()),
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.output_language, None);
    }

    #[test]
    fn normalize_adapter_timeout_clamps_minimum() {
        let mut config = Config {
            adapter_timeout_secs: Some(2),
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.adapter_timeout_secs, Some(5));
    }

    #[test]
    fn normalize_adapter_retry_delay_clamps_minimum() {
        let mut config = Config {
            adapter_retry_delay_ms: Some(10),
            ..Config::default()
        };
        config.normalize();
        assert_eq!(config.adapter_retry_delay_ms, Some(50));
    }

    #[test]
    fn normalize_feedback_suppression_zero_resets() {
        let mut config = Config {
            feedback_suppression_threshold: 0,
            ..Config::default()
        };
        config.normalize();
        assert_eq!(
            config.feedback_suppression_threshold,
            default_feedback_suppression_threshold()
        );
    }

    #[test]
    fn test_apply_cli_overrides() {
        let mut config = Config::default();
        config.apply_cli_overrides(CliOverrides {
            temperature: Some(0.5),
            max_tokens: Some(8000),
            strictness: Some(3),
            comment_types: Some(vec!["logic".to_string()]),
            openai_responses: Some(true),
            base_url: Some("http://localhost:1234".to_string()),
            api_key: Some("test-key".to_string()),
            adapter: Some("openai".to_string()),
            timeout: Some(60),
            max_retries: Some(5),
            file_change_limit: Some(10),
            output_language: Some("ja".to_string()),
            ..Default::default()
        });
        assert!((config.temperature - 0.5).abs() < f32::EPSILON);
        assert_eq!(config.max_tokens, 8000);
        assert_eq!(config.strictness, 3);
        assert_eq!(config.comment_types, vec!["logic".to_string()]);
        assert_eq!(config.openai_use_responses, Some(true));
        assert_eq!(config.base_url.as_deref(), Some("http://localhost:1234"));
        assert_eq!(config.api_key.as_deref(), Some("test-key"));
        assert_eq!(config.adapter.as_deref(), Some("openai"));
        assert_eq!(config.adapter_timeout_secs, Some(60));
        assert_eq!(config.adapter_max_retries, Some(5));
        assert_eq!(config.file_change_limit, Some(10));
        assert_eq!(config.output_language.as_deref(), Some("ja"));
    }

    #[test]
    fn test_apply_cli_overrides_nones_dont_change() {
        let mut config = Config::default();
        let orig_temp = config.temperature;
        let orig_tokens = config.max_tokens;
        config.apply_cli_overrides(CliOverrides::default());
        assert!((config.temperature - orig_temp).abs() < f32::EPSILON);
        assert_eq!(config.max_tokens, orig_tokens);
    }

    #[test]
    fn test_apply_cli_overrides_lsp() {
        let mut config = Config::default();
        config.apply_cli_overrides(CliOverrides {
            lsp_command: Some("rust-analyzer".to_string()),
            ..Default::default()
        });
        assert!(config.symbol_index);
        assert_eq!(config.symbol_index_provider, "lsp");
        assert_eq!(
            config.symbol_index_lsp_command.as_deref(),
            Some("rust-analyzer")
        );
    }

    #[test]
    fn test_model_role_primary_returns_model() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            ..Config::default()
        };
        assert_eq!(
            config.model_for_role(ModelRole::Primary),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn test_model_role_weak_fallback_to_primary() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_weak: None,
            ..Config::default()
        };
        assert_eq!(config.model_for_role(ModelRole::Weak), "claude-sonnet-4-6");
    }

    #[test]
    fn test_model_role_weak_explicit() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_weak: Some("claude-haiku-4-5".to_string()),
            ..Config::default()
        };
        assert_eq!(config.model_for_role(ModelRole::Weak), "claude-haiku-4-5");
    }

    #[test]
    fn test_model_role_reasoning_fallback() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_reasoning: None,
            ..Config::default()
        };
        assert_eq!(
            config.model_for_role(ModelRole::Reasoning),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn test_model_role_reasoning_explicit() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_reasoning: Some("claude-opus-4-6".to_string()),
            ..Config::default()
        };
        assert_eq!(
            config.model_for_role(ModelRole::Reasoning),
            "claude-opus-4-6"
        );
    }

    #[test]
    fn test_model_role_embedding_default() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_embedding: None,
            ..Config::default()
        };
        // Falls back to primary model when no embedding model configured
        assert_eq!(
            config.model_for_role(ModelRole::Embedding),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn test_model_role_embedding_explicit() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_embedding: Some("custom-embedding-model".to_string()),
            ..Config::default()
        };
        assert_eq!(
            config.model_for_role(ModelRole::Embedding),
            "custom-embedding-model"
        );
    }

    #[test]
    fn test_model_role_fast_fallback_to_primary() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_fast: None,
            model_weak: None,
            ..Config::default()
        };
        assert_eq!(config.model_for_role(ModelRole::Fast), "claude-sonnet-4-6");
    }

    #[test]
    fn test_model_role_fast_fallback_to_weak() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_fast: None,
            model_weak: Some("claude-haiku-4-5".to_string()),
            ..Config::default()
        };
        assert_eq!(config.model_for_role(ModelRole::Fast), "claude-haiku-4-5");
    }

    #[test]
    fn test_model_role_fast_explicit() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_fast: Some("gpt-4o-mini".to_string()),
            model_weak: Some("claude-haiku-4-5".to_string()),
            ..Config::default()
        };
        assert_eq!(config.model_for_role(ModelRole::Fast), "gpt-4o-mini");
    }

    #[test]
    fn test_to_model_config_for_role_fast() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_fast: Some("gpt-4o-mini".to_string()),
            ..Config::default()
        };
        let fast_config = config.to_model_config_for_role(ModelRole::Fast);
        assert_eq!(fast_config.model_name, "gpt-4o-mini");
    }

    #[test]
    fn test_to_model_config_for_role_uses_correct_model() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_weak: Some("claude-haiku-4-5".to_string()),
            ..Config::default()
        };
        let primary_config = config.to_model_config_for_role(ModelRole::Primary);
        assert_eq!(primary_config.model_name, "claude-sonnet-4-6");

        let weak_config = config.to_model_config_for_role(ModelRole::Weak);
        assert_eq!(weak_config.model_name, "claude-haiku-4-5");
    }

    #[test]
    fn test_generation_and_auditing_roles_default() {
        let config = Config::default();
        assert_eq!(config.generation_model_role, ModelRole::Primary);
        assert_eq!(config.auditing_model_role, ModelRole::Reasoning);
        assert_eq!(config.generation_model_name(), config.model.as_str());
        assert_eq!(config.auditing_model_name(), "anthropic/claude-opus-4.5");
    }

    #[test]
    fn test_config_deserialize_model_routing_roles_from_yaml() {
        let yaml = r#"
model: claude-sonnet-4-6
model_reasoning: claude-opus-4-6
generation_model_role: reasoning
auditing_model_role: primary
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.generation_model_role, ModelRole::Reasoning);
        assert_eq!(config.auditing_model_role, ModelRole::Primary);
        assert_eq!(config.generation_model_name(), "claude-opus-4-6");
        assert_eq!(config.auditing_model_name(), "claude-sonnet-4-6");
    }

    #[test]
    fn test_set_model_for_role_updates_selected_slot() {
        let mut config = Config::default();
        config.set_model_for_role(ModelRole::Reasoning, "openai/o3");
        config.set_model_for_role(ModelRole::Fast, "openai/gpt-4.1-mini");

        assert_eq!(config.model_reasoning.as_deref(), Some("openai/o3"));
        assert_eq!(config.model_fast.as_deref(), Some("openai/gpt-4.1-mini"));
    }

    #[test]
    fn test_inferred_provider_label_for_role_uses_model_prefix() {
        let config = Config {
            model_reasoning: Some("openai/o3".to_string()),
            ..Config::default()
        };

        assert_eq!(
            config
                .inferred_provider_label_for_role(ModelRole::Reasoning)
                .as_deref(),
            Some("openrouter")
        );
        assert_eq!(
            config
                .inferred_provider_label_for_role(ModelRole::Primary)
                .as_deref(),
            Some("anthropic")
        );
    }

    #[test]
    fn test_to_model_config_for_role_uses_provider_specific_credentials() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_reasoning: Some("openai/o3".to_string()),
            api_key: Some("legacy-key".to_string()),
            base_url: Some("https://api.anthropic.com".to_string()),
            providers: HashMap::from([
                (
                    "anthropic".to_string(),
                    ProviderConfig {
                        api_key: Some("anthropic-key".to_string()),
                        base_url: Some("https://api.anthropic.com".to_string()),
                        enabled: true,
                    },
                ),
                (
                    "openrouter".to_string(),
                    ProviderConfig {
                        api_key: Some("openrouter-key".to_string()),
                        base_url: Some("https://openrouter.ai/api/v1".to_string()),
                        enabled: true,
                    },
                ),
            ]),
            ..Config::default()
        };

        let primary = config.to_model_config_for_role(ModelRole::Primary);
        assert_eq!(primary.api_key.as_deref(), Some("anthropic-key"));
        assert_eq!(
            primary.base_url.as_deref(),
            Some("https://api.anthropic.com")
        );

        let reasoning = config.to_model_config_for_role(ModelRole::Reasoning);
        assert_eq!(reasoning.model_name, "openai/o3");
        assert_eq!(reasoning.api_key.as_deref(), Some("openrouter-key"));
        assert_eq!(
            reasoning.base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
    }

    #[test]
    fn test_validation_issues_warn_for_mixed_provider_legacy_fallbacks_and_partial_integrations() {
        let config = Config {
            model: "claude-sonnet-4-6".to_string(),
            model_reasoning: Some("openai/o3".to_string()),
            api_key: Some("legacy-key".to_string()),
            github: GitHubConfig {
                app_id: Some(42),
                ..GitHubConfig::default()
            },
            jira: JiraConfig {
                base_url: Some("https://jira.example.com".to_string()),
                ..JiraConfig::default()
            },
            vault: VaultConfig {
                addr: Some("https://vault.example.com".to_string()),
                ..VaultConfig::default()
            },
            ..Config::default()
        };

        let issues = config.validation_issues();
        assert!(issues.iter().any(|issue| issue.field == "providers"));
        assert!(issues.iter().any(|issue| issue.field == "github_app"));
        assert!(issues.iter().any(|issue| issue.field == "jira"));
        assert!(issues.iter().any(|issue| issue.field == "vault"));
    }

    #[test]
    fn test_github_auto_review_events_default_to_opened_and_synchronize() {
        let config = Config::default();

        assert_eq!(
            config.github.auto_review_events,
            vec!["opened".to_string(), "synchronize".to_string()]
        );
    }

    #[test]
    fn test_config_deserialize_github_review_request_controls() {
        let config: Config = serde_yaml::from_str(
            r#"
github_auto_review_events:
  - review_requested
github_review_request_reviewers:
  - EvalOpsBot
"#,
        )
        .unwrap();

        assert_eq!(
            config.github.auto_review_events,
            vec!["review_requested".to_string()]
        );
        assert_eq!(
            config.github.review_request_reviewers,
            vec!["EvalOpsBot".to_string()]
        );
    }

    #[test]
    fn test_normalize_github_review_request_controls() {
        let mut config = Config {
            github: GitHubConfig {
                auto_review_events: vec![
                    " Review_Requested ".to_string(),
                    "review_requested".to_string(),
                    "opened".to_string(),
                    "".to_string(),
                ],
                review_request_reviewers: vec![
                    " EvalOpsBot ".to_string(),
                    "evalopsbot".to_string(),
                    "AnotherBot".to_string(),
                    "".to_string(),
                ],
                ..GitHubConfig::default()
            },
            ..Config::default()
        };

        config.normalize();

        assert_eq!(
            config.github.auto_review_events,
            vec!["review_requested".to_string(), "opened".to_string()]
        );
        assert_eq!(
            config.github.review_request_reviewers,
            vec!["EvalOpsBot".to_string(), "AnotherBot".to_string()]
        );
    }

    #[test]
    fn test_validation_warns_when_review_request_enabled_without_reviewers() {
        let config = Config {
            github: GitHubConfig {
                auto_review_events: vec!["review_requested".to_string()],
                review_request_reviewers: Vec::new(),
                ..GitHubConfig::default()
            },
            ..Config::default()
        };

        let issues = config.validation_issues();
        assert!(issues
            .iter()
            .any(|issue| issue.field == "github_review_request_reviewers"));
    }

    #[test]
    fn test_normalize_rejects_embedding_for_generation_verification_and_auditing() {
        let mut config = Config {
            generation_model_role: ModelRole::Embedding,
            auditing_model_role: ModelRole::Embedding,
            verification: VerificationConfig {
                model_role: ModelRole::Embedding,
                additional_model_roles: vec![
                    ModelRole::Embedding,
                    ModelRole::Reasoning,
                    ModelRole::Reasoning,
                ],
                ..VerificationConfig::default()
            },
            ..Config::default()
        };

        config.normalize();

        assert_eq!(config.generation_model_role, ModelRole::Primary);
        assert_eq!(config.auditing_model_role, ModelRole::Reasoning);
        assert_eq!(config.verification.model_role, ModelRole::Weak);
        assert_eq!(
            config.verification.additional_model_roles,
            vec![ModelRole::Reasoning]
        );
    }

    #[test]
    fn test_fallback_models_default_empty() {
        let config = Config::default();
        assert!(config.fallback_models.is_empty());
    }

    #[test]
    fn test_config_deserialization_with_model_roles() {
        let yaml = r#"
model: claude-sonnet-4-6
model_weak: claude-haiku-4-5
model_reasoning: claude-opus-4-6
model_embedding: text-embedding-3-small
fallback_models:
  - gpt-4o
  - claude-sonnet-4-6
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.model, "claude-sonnet-4-6");
        assert_eq!(config.model_weak, Some("claude-haiku-4-5".to_string()));
        assert_eq!(config.model_reasoning, Some("claude-opus-4-6".to_string()));
        assert_eq!(
            config.model_embedding,
            Some("text-embedding-3-small".to_string())
        );
        assert_eq!(config.fallback_models.len(), 2);
    }

    #[test]
    fn test_config_deserialization_without_model_roles() {
        // Existing configs without new fields should still work
        let yaml = r#"
model: claude-sonnet-4-6
temperature: 0.3
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.model, "claude-sonnet-4-6");
        assert_eq!(
            config.model_weak,
            Some("anthropic/claude-sonnet-4.5".to_string())
        );
        assert_eq!(
            config.model_fast,
            Some("anthropic/claude-sonnet-4.5".to_string())
        );
        assert_eq!(
            config.model_reasoning,
            Some("anthropic/claude-opus-4.5".to_string())
        );
        assert!(config.model_embedding.is_none());
        assert!(config.fallback_models.is_empty());
    }

    #[test]
    fn test_default_model_is_frontier() {
        // Per CLAUDE.md: "Always use frontier models (Opus) for AI-powered features
        // — never default to Sonnet or smaller"
        let model = default_model();
        assert!(
            model.contains("opus"),
            "Default model should be Opus (frontier), got: {model}"
        );
    }

    #[test]
    fn test_config_deserialize_verification_enum_fields_from_yaml() {
        // Regression (PR37): flattened verification_model_role / verification_consensus_mode
        // must deserialize from YAML. serde_yaml can have issues with enums in flattened
        // structs; this test locks in that config.verification (flatten) works.
        let yaml = r#"
model: claude-sonnet-4-6
verification_model_role: weak
verification_consensus_mode: majority
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.verification.model_role, ModelRole::Weak);
        assert_eq!(
            config.verification.consensus_mode,
            VerificationConsensusMode::Majority
        );
    }

    #[test]
    fn test_config_deserialize_verification_enum_primary_and_all() {
        // TDD: prove enum variants deserialize from YAML (primary, all).
        let yaml = r#"
model: claude-sonnet-4-6
verification_model_role: primary
verification_consensus_mode: all
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.verification.model_role, ModelRole::Primary);
        assert_eq!(
            config.verification.consensus_mode,
            VerificationConsensusMode::All
        );
    }

    #[test]
    fn test_config_deserialize_review_rules_prose_from_yaml() {
        // #12: natural language rules — list of strings in YAML
        let yaml = r#"
model: claude-sonnet-4-6
review_rules_prose:
  - Always use parameterized queries.
  - No direct SQL string concatenation.
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let rules = config.review_rules_prose.as_ref().unwrap();
        assert_eq!(rules.len(), 2);
        assert!(rules[0].contains("parameterized"));
        assert!(rules[1].contains("SQL"));
    }

    #[test]
    fn test_config_deserialize_triage_skip_deletion_only() {
        // #29: optional skip deletion-only diffs
        let yaml = r#"
model: claude-sonnet-4-6
triage_skip_deletion_only: true
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(config.triage_skip_deletion_only);
    }

    #[test]
    fn test_config_default_triage_skip_deletion_only_false() {
        let config = Config::default();
        assert!(
            !config.triage_skip_deletion_only,
            "default: deletions get review unless explicitly enabled"
        );
    }

    #[test]
    fn test_config_deserialize_triage_skip_deletion_only_false() {
        let yaml = r#"
model: claude-sonnet-4-6
triage_skip_deletion_only: false
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(!config.triage_skip_deletion_only);
    }

    #[test]
    fn test_default_frontier_role_models_match_requested_pair() {
        let config = Config::default();
        assert_eq!(config.model, "anthropic/claude-opus-4.5");
        assert_eq!(
            config.model_weak,
            Some("anthropic/claude-sonnet-4.5".to_string())
        );
        assert_eq!(
            config.model_fast,
            Some("anthropic/claude-sonnet-4.5".to_string())
        );
        assert_eq!(
            config.model_reasoning,
            Some("anthropic/claude-opus-4.5".to_string())
        );
        assert_eq!(config.verification.model_role, ModelRole::Weak);
        assert_eq!(
            config.verification.additional_model_roles,
            vec![ModelRole::Reasoning]
        );
        assert_eq!(
            config.verification.consensus_mode,
            VerificationConsensusMode::Any
        );
    }

    #[test]
    fn test_is_local_endpoint_openrouter_not_local() {
        // BUG: Config::is_local_endpoint returns true for any URL that doesn't
        // contain "openai.com" or "anthropic.com", including cloud providers
        let config = Config {
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            ..Default::default()
        };
        assert!(
            !config.is_local_endpoint(),
            "OpenRouter is a cloud provider, not a local endpoint"
        );
    }

    #[test]
    fn test_is_local_endpoint_azure_not_local() {
        let config = Config {
            base_url: Some("https://myinstance.openai.azure.com/v1".to_string()),
            ..Default::default()
        };
        assert!(
            !config.is_local_endpoint(),
            "Azure OpenAI is a cloud provider, not a local endpoint"
        );
    }

    // --- agent_tools_enabled tests ---

    #[test]
    fn test_agent_tools_enabled_default_is_none() {
        let config = Config::default();
        assert!(
            config.agent.tools_enabled.is_none(),
            "Default should be None (all tools enabled)"
        );
    }

    #[test]
    fn test_agent_tools_enabled_serialize_none() {
        // When None, the field serializes as null (consistent with other Option fields)
        let config = Config::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(
            yaml.contains("agent_tools_enabled"),
            "Field should be present in serialized YAML even when None"
        );
    }

    #[test]
    fn test_agent_tools_enabled_serialize_some() {
        let config = Config {
            agent: AgentConfig {
                tools_enabled: Some(vec!["read_file".to_string(), "search_code".to_string()]),
                ..AgentConfig::default()
            },
            ..Config::default()
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(yaml.contains("agent_tools_enabled"));
        assert!(yaml.contains("read_file"));
        assert!(yaml.contains("search_code"));
    }

    #[test]
    fn test_agent_tools_enabled_deserialize_missing_field() {
        // Existing configs without the field should deserialize with None
        let yaml = r#"
model: claude-opus-4-6
temperature: 0.3
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(config.agent.tools_enabled.is_none());
    }

    #[test]
    fn test_agent_tools_enabled_deserialize_explicit_list() {
        let yaml = r#"
model: claude-opus-4-6
agent_tools_enabled:
  - read_file
  - search_code
  - list_files
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let tools = config.agent.tools_enabled.unwrap();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0], "read_file");
        assert_eq!(tools[1], "search_code");
        assert_eq!(tools[2], "list_files");
    }

    #[test]
    fn test_agent_tools_enabled_deserialize_empty_list() {
        let yaml = r#"
model: claude-opus-4-6
agent_tools_enabled: []
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let tools = config.agent.tools_enabled.unwrap();
        assert!(
            tools.is_empty(),
            "Empty list should deserialize as Some([])"
        );
    }

    #[test]
    fn test_agent_tools_enabled_round_trip() {
        let original = Config {
            agent: AgentConfig {
                tools_enabled: Some(vec!["read_file".to_string(), "search_code".to_string()]),
                ..AgentConfig::default()
            },
            ..Config::default()
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let restored: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original.agent.tools_enabled, restored.agent.tools_enabled);
    }

    #[test]
    fn test_agent_tools_enabled_round_trip_none() {
        let original = Config::default();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let restored: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original.agent.tools_enabled, restored.agent.tools_enabled);
    }

    #[test]
    fn test_retention_defaults_are_present() {
        let config = Config::default();
        assert_eq!(config.retention.review_max_age_days, 30);
        assert_eq!(config.retention.review_max_count, 1000);
        assert_eq!(config.retention.eval_artifact_max_age_days, 30);
        assert_eq!(config.retention.trend_history_max_entries, 200);
    }

    #[test]
    fn test_retention_normalize_resets_invalid_values() {
        let mut config = Config {
            retention: RetentionConfig {
                review_max_age_days: 0,
                review_max_count: 0,
                eval_artifact_max_age_days: -7,
                trend_history_max_entries: 0,
            },
            ..Config::default()
        };

        config.normalize();

        assert_eq!(config.retention.review_max_age_days, 30);
        assert_eq!(config.retention.review_max_count, 1000);
        assert_eq!(config.retention.eval_artifact_max_age_days, 30);
        assert_eq!(config.retention.trend_history_max_entries, 200);
    }
}
