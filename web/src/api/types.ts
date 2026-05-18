export type Severity = 'Error' | 'Warning' | 'Info' | 'Suggestion'
export type Category = 'Bug' | 'Security' | 'Performance' | 'Style' | 'Documentation' | 'BestPractice' | 'Maintainability' | 'Testing' | 'Architecture'
export type FixEffort = 'Low' | 'Medium' | 'High'
export type ReviewStatus = 'Pending' | 'Running' | 'Complete' | 'Failed'
export type FeedbackAction = 'accept' | 'reject'
export type CommentLifecycleStatus = 'Open' | 'Resolved' | 'Dismissed'
export type CommentLifecycleAction = 'open' | 'resolved' | 'dismissed'
export type CommentOutcome = 'new' | 'accepted' | 'rejected' | 'addressed' | 'stale' | 'auto_fixed'
export type MergeReadiness = 'Ready' | 'NeedsAttention' | 'NeedsReReview'
export type ReviewVerificationState = 'NotApplicable' | 'Verified' | 'Inconclusive'

export interface CodeSuggestion {
  original_code: string
  suggested_code: string
  explanation: string
  diff: string
}

export interface Comment {
  id: string
  file_path: string
  line_number: number
  content: string
  rule_id?: string
  severity: Severity
  category: Category
  suggestion?: string
  confidence: number
  code_suggestion?: CodeSuggestion
  tags: string[]
  fix_effort: FixEffort
  feedback?: FeedbackAction
  feedback_explanation?: string
  status?: CommentLifecycleStatus
  resolved_at?: string | number
  outcomes?: CommentOutcome[]
}

export interface ReviewSummary {
  total_comments: number
  by_severity: Record<string, number>
  by_category: Record<string, number>
  critical_issues: number
  files_reviewed: number
  overall_score: number
  recommendations: string[]
  open_comments: number
  open_by_severity: Record<string, number>
  open_blocking_comments: number
  open_informational_comments: number
  resolved_comments: number
  dismissed_comments: number
  open_blockers: number
  completeness: {
    total_findings: number
    acknowledged_findings: number
    fixed_findings: number
    stale_findings: number
  }
  merge_readiness: MergeReadiness
  verification: {
    state: ReviewVerificationState
    judge_count: number
    required_votes: number
    warning_count: number
    filtered_comments: number
    abstained_comments: number
  }
  readiness_reasons: string[]
  loop_telemetry?: {
    iterations: number
    fixes_attempted: number
    findings_cleared: number
    findings_reopened: number
  }
}

export interface FileMetricEvent {
  file_path: string
  latency_ms: number
  prompt_tokens: number
  completion_tokens: number
  total_tokens: number
  comment_count: number
}

export interface HotspotDetail {
  file_path: string
  risk_score: number
  reasons: string[]
}

export interface CostBreakdownRow {
  workload: string
  role: string
  provider?: string
  model: string
  prompt_tokens: number
  completion_tokens: number
  total_tokens: number
  cost_estimate_usd: number
}

export interface ReviewEvent {
  review_id: string
  event_type: string
  diff_source: string
  title?: string
  model: string
  provider?: string
  base_url?: string
  duration_ms: number
  diff_fetch_ms?: number
  llm_total_ms?: number
  diff_bytes: number
  diff_files_total: number
  diff_files_reviewed: number
  diff_files_skipped: number
  comments_total: number
  comments_by_severity: Record<string, number>
  comments_by_category: Record<string, number>
  overall_score?: number
  hotspots_detected: number
  high_risk_files: number
  tokens_prompt?: number
  tokens_completion?: number
  tokens_total?: number
  cost_estimate_usd?: number
  cost_breakdowns?: CostBreakdownRow[]
  file_metrics?: FileMetricEvent[]
  hotspot_details?: HotspotDetail[]
  convention_suppressed?: number
  comments_by_pass?: Record<string, number>
  agent_iterations?: number
  agent_tool_calls?: AgentToolCallEvent[]
  github_posted: boolean
  github_repo?: string
  github_pr?: number
  error?: string
  created_at?: string
}

export interface ModelStats {
  model: string
  count: number
  avg_duration_ms: number
  total_tokens: number
  avg_score?: number
}

export interface SourceStats {
  source: string
  count: number
}

export interface RepoStats {
  repo: string
  count: number
  avg_score?: number
}

export interface DailyCount {
  date: string
  completed: number
  failed: number
}

export interface EventStats {
  total_reviews: number
  completed_count: number
  failed_count: number
  total_tokens: number
  avg_duration_ms: number
  avg_score?: number
  error_rate: number
  p50_latency_ms: number
  p95_latency_ms: number
  p99_latency_ms: number
  by_model: ModelStats[]
  by_source: SourceStats[]
  by_repo: RepoStats[]
  severity_totals: Record<string, number>
  category_totals: Record<string, number>
  daily_counts: DailyCount[]
  total_cost_estimate: number
  cost_breakdowns?: CostBreakdownRow[]
}

export interface ReviewProgress {
  current_file?: string
  files_total: number
  files_completed: number
  files_skipped: number
  elapsed_ms: number
  estimated_remaining_ms?: number
}

export interface ReviewSession {
  id: string
  status: ReviewStatus
  diff_source: string
  started_at: string | number
  completed_at?: string | number
  comments: Comment[]
  summary?: ReviewSummary
  files_reviewed: number
  error?: string
  diff_content?: string
  event?: ReviewEvent
  progress?: ReviewProgress
}

export interface EvalTrendEntry {
  timestamp: string
  micro_f1: number
  micro_precision: number
  micro_recall: number
  fixture_count: number
  label?: string
  weighted_score?: number
  model?: string
  provider?: string
  review_mode?: string
  comparison_group?: string
  pass_rate?: number
  lifecycle_accuracy?: number
  usefulness_score?: number
  suite_micro_f1: Record<string, number>
  category_micro_f1: Record<string, number>
  language_micro_f1: Record<string, number>
  verification_warning_count?: number
  verification_fail_open_count?: number
  verification_parse_failure_count?: number
  verification_request_failure_count?: number
  verification_verified_checks?: number
  verification_total_checks?: number
  verification_verified_pct?: number
  cost_breakdowns?: CostBreakdownRow[]
}

export interface EvalQualityTrend {
  entries: EvalTrendEntry[]
}

export interface FeedbackEvalTrendGap {
  name: string
  feedback_total: number
  high_confidence_total: number
  high_confidence_acceptance_rate: number
  eval_score?: number
  gap?: number
}

export interface FeedbackEvalTrendEntry {
  timestamp: string
  labeled_comments: number
  accepted: number
  rejected: number
  acceptance_rate: number
  confidence_threshold: number
  confidence_agreement_rate?: number
  confidence_precision?: number
  confidence_recall?: number
  confidence_f1?: number
  eval_label?: string
  eval_model?: string
  eval_provider?: string
  attention_by_category: FeedbackEvalTrendGap[]
  attention_by_rule: FeedbackEvalTrendGap[]
}

export interface FeedbackEvalTrend {
  entries: FeedbackEvalTrendEntry[]
}

export interface AnalyticsTrendsResponse {
  eval_trend_path: string
  feedback_eval_trend_path: string
  eval_trend: EvalQualityTrend
  feedback_eval_trend: FeedbackEvalTrend
  warnings: string[]
}

export interface StatusResponse {
  repo_path: string
  branch?: string
  model: string
  adapter?: string
  base_url?: string
  active_reviews: number
}

export interface DoctorModel {
  name: string
  size_mb: number
  quantization?: string
  family?: string
  parameter_size?: string
}

export interface DoctorResponse {
  config: {
    model: string
    adapter?: string
    base_url: string
    api_key_set: boolean
    context_window?: number
  }
  learning?: {
    enhanced_feedback: boolean
    semantic_feedback: boolean
    semantic_rag: boolean
    feedback_path: string
    semantic_feedback_path: string
    feedback_store_exists: boolean
    semantic_feedback_store_exists: boolean
    min_feedback_observations: number
    semantic_feedback_min_examples: number
    semantic_feedback_similarity: number
  }
  endpoint_reachable: boolean
  endpoint_type?: string
  models: DoctorModel[]
  recommended_model?: string
}

export interface StartReviewRequest {
  diff_source: 'head' | 'staged' | 'branch'
  base_branch?: string
  model?: string
  strictness?: number
  review_profile?: string
}

// Parsed diff structures for the viewer
export interface DiffFile {
  path: string
  oldPath?: string
  hunks: DiffHunk[]
  status: 'added' | 'modified' | 'deleted' | 'renamed'
}

export interface DiffHunk {
  header: string
  oldStart: number
  oldCount: number
  newStart: number
  newCount: number
  lines: DiffLine[]
}

export interface DiffLine {
  type: 'add' | 'del' | 'context'
  content: string
  oldNumber?: number
  newNumber?: number
}

export interface ProviderConfig {
  api_key?: string
  base_url?: string
  enabled: boolean
}

export interface TestProviderRequest {
  provider: string
  api_key?: string
  base_url?: string
}

export interface TestProviderResponse {
  ok: boolean
  message: string
  models: string[]
}

export interface GhStatusResponse {
  authenticated: boolean
  username?: string
  avatar_url?: string
  scopes: string[]
}

export interface DeviceFlowResponse {
  device_code: string
  user_code: string
  verification_uri: string
  expires_in: number
  interval: number
}

export interface PollDeviceFlowResponse {
  authenticated: boolean
  username?: string
  avatar_url?: string
  error?: string
}

export interface WebhookStatusResponse {
  configured: boolean
  url: string
}

export interface GhRepo {
  full_name: string
  description: string | null
  language: string | null
  updated_at: string
  open_prs: number
  open_blockers?: number
  blocking_prs?: number
  default_branch: string
  stargazers_count: number
  private: boolean
}

export interface GhPullRequest {
  number: number
  title: string
  author: string
  state: string
  created_at: string
  updated_at: string
  additions: number
  deletions: number
  changed_files: number
  head_branch: string
  base_branch: string
  labels: string[]
  draft: boolean
  open_blockers?: number
  merge_readiness?: MergeReadiness
}

export interface PrReadinessReview {
  id: string
  status: ReviewStatus
  started_at: string | number
  completed_at?: string | number
  reviewed_head_sha?: string
  summary?: ReviewSummary
  files_reviewed: number
  comment_count: number
  error?: string
}

export interface PrReadinessSnapshot {
  repo: string
  pr_number: number
  diff_source: string
  current_head_sha?: string
  latest_review?: PrReadinessReview
  timeline?: PrReadinessReview[]
}

export interface StartPrReviewRequest {
  repo: string
  pr_number: number
  post_results: boolean
}

export interface AgentToolInfo {
  name: string
  description: string
  requires?: string
}

export interface AgentToolCallEvent {
  iteration: number
  tool_name: string
  duration_ms: number
}
