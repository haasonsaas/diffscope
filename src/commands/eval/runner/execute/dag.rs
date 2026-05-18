use anyhow::Result;
use futures::{future::BoxFuture, FutureExt};
use std::cell::RefCell;
use tracing::debug;

use crate::config;
use crate::core;
use crate::core::dag::{
    describe_dag, execute_dag_with_parallelism, DagExecutionRecord, DagExecutionTrace,
    DagGraphContract, DagNode, DagNodeContract, DagNodeExecutionHints, DagNodeKind, DagNodeSpec,
};
use crate::core::eval_benchmarks::FixtureResult as BenchmarkFixtureResult;
use crate::review::review_diff_content_raw;

use super::super::super::{EvalAgentActivity, EvalReproductionSummary, EvalVerificationReport};
use super::super::matching::{evaluate_fixture_expectations, FixtureMatchSummary};
use super::artifact::{
    maybe_write_fixture_artifact, EvalFixtureArtifactContext, EvalFixtureArtifactInput,
};
use super::loading::PreparedFixtureExecution;
use super::repro::maybe_run_reproduction_validation;
use super::result::{
    append_review_summary_failures, append_total_comment_failures, build_benchmark_metrics,
    convert_agent_activity, convert_verification_report, FixtureResultDetails,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum EvalFixtureStage {
    Review,
    ExpectationMatching,
    CommentCountValidation,
    BenchmarkMetrics,
    ReproductionValidation,
    ArtifactCapture,
}

impl DagNode for EvalFixtureStage {
    fn name(&self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::ExpectationMatching => "expectation_matching",
            Self::CommentCountValidation => "comment_count_validation",
            Self::BenchmarkMetrics => "benchmark_metrics",
            Self::ReproductionValidation => "reproduction_validation",
            Self::ArtifactCapture => "artifact_capture",
        }
    }
}

pub(super) struct EvalFixtureDagConfig {
    pub(super) repro_validate: bool,
    pub(super) repro_max_comments: usize,
    pub(super) artifact_context: Option<EvalFixtureArtifactContext>,
}

pub(super) struct EvalFixtureExecutionOutcome {
    pub(super) prepared: PreparedFixtureExecution,
    pub(super) total_comments: usize,
    pub(super) match_summary: FixtureMatchSummary,
    pub(super) benchmark_metrics: Option<BenchmarkFixtureResult>,
    pub(super) details: FixtureResultDetails,
}

struct EvalFixtureDagContext {
    prepared: PreparedFixtureExecution,
    dag_config: EvalFixtureDagConfig,
    comments: Vec<core::Comment>,
    warnings: Vec<String>,
    cost_breakdowns: Vec<crate::server::cost::CostBreakdownRow>,
    verification_report: Option<EvalVerificationReport>,
    agent_activity: Option<EvalAgentActivity>,
    reproduction_summary: Option<EvalReproductionSummary>,
    total_comments: usize,
    match_summary: Option<FixtureMatchSummary>,
    failures: Vec<String>,
    benchmark_metrics: Option<BenchmarkFixtureResult>,
    artifact_path: Option<String>,
    dag_traces: Vec<DagExecutionTrace>,
}

#[derive(Debug)]
enum EvalFixtureStageOutput {
    Review {
        comments: Vec<core::Comment>,
        warnings: Vec<String>,
        cost_breakdowns: Vec<crate::server::cost::CostBreakdownRow>,
        verification_report: Option<EvalVerificationReport>,
        agent_activity: Option<EvalAgentActivity>,
        dag_traces: Vec<DagExecutionTrace>,
    },
    ExpectationMatching {
        match_summary: FixtureMatchSummary,
        failures: Vec<String>,
    },
    CommentCountValidation {
        failures: Vec<String>,
    },
    BenchmarkMetrics {
        benchmark_metrics: Option<BenchmarkFixtureResult>,
    },
    ReproductionValidation {
        reproduction_summary: Option<EvalReproductionSummary>,
        warnings: Vec<String>,
        cost_breakdowns: Vec<crate::server::cost::CostBreakdownRow>,
    },
    ArtifactCapture {
        artifact_path: Option<String>,
    },
}

impl EvalFixtureDagContext {
    fn new(prepared: PreparedFixtureExecution, dag_config: EvalFixtureDagConfig) -> Self {
        Self {
            prepared,
            dag_config,
            comments: Vec::new(),
            warnings: Vec::new(),
            cost_breakdowns: Vec::new(),
            verification_report: None,
            agent_activity: None,
            reproduction_summary: None,
            total_comments: 0,
            match_summary: None,
            failures: Vec::new(),
            benchmark_metrics: None,
            artifact_path: None,
            dag_traces: Vec::new(),
        }
    }

    fn into_outcome(
        self,
        eval_records: Vec<DagExecutionRecord>,
    ) -> Result<EvalFixtureExecutionOutcome> {
        let mut dag_traces = self.dag_traces;
        dag_traces.push(DagExecutionTrace {
            graph_name: "eval_fixture_execution".to_string(),
            records: eval_records,
        });
        Ok(EvalFixtureExecutionOutcome {
            prepared: self.prepared,
            total_comments: self.total_comments,
            match_summary: self.match_summary.ok_or_else(|| {
                anyhow::anyhow!("fixture DAG did not produce expectation matches")
            })?,
            benchmark_metrics: self.benchmark_metrics,
            details: FixtureResultDetails {
                warnings: self.warnings,
                verification_report: self.verification_report,
                agent_activity: self.agent_activity,
                reproduction_summary: self.reproduction_summary,
                artifact_path: self.artifact_path,
                failures: self.failures,
                cost_breakdowns: self.cost_breakdowns,
                dag_traces,
            },
        })
    }
}

pub(super) async fn execute_eval_fixture_dag(
    config: &config::Config,
    prepared: PreparedFixtureExecution,
    dag_config: EvalFixtureDagConfig,
) -> Result<EvalFixtureExecutionOutcome> {
    let specs = build_stage_specs(dag_config.repro_validate);
    let dag_description = describe_dag(&specs);
    debug!(?dag_description, "Executing eval fixture DAG");
    let context = RefCell::new(EvalFixtureDagContext::new(prepared, dag_config));
    let records = execute_dag_with_parallelism(
        &specs,
        |stage| {
            let mut context = context.borrow_mut();
            spawn_stage(stage, config, &mut context)
        },
        |stage, output| {
            let mut context = context.borrow_mut();
            apply_stage_output(stage, output, &mut context)
        },
    )
    .await?;
    let mut context = context.into_inner();
    rewrite_fixture_artifact_with_eval_trace(&mut context, &records).await?;

    context.into_outcome(records)
}

fn stage_hints(stage: EvalFixtureStage) -> DagNodeExecutionHints {
    match stage {
        EvalFixtureStage::Review => DagNodeExecutionHints {
            parallelizable: false,
            retryable: true,
            timeout_ms: None,
            side_effects: false,
            subgraph: Some("review_pipeline".to_string()),
        },
        // Linear chain (each depends on previous); parallelizable kept for consistency / future DAG shape.
        EvalFixtureStage::ExpectationMatching
        | EvalFixtureStage::CommentCountValidation
        | EvalFixtureStage::BenchmarkMetrics => DagNodeExecutionHints {
            parallelizable: true,
            retryable: true,
            timeout_ms: None,
            side_effects: false,
            subgraph: None,
        },
        EvalFixtureStage::ReproductionValidation => DagNodeExecutionHints {
            parallelizable: true,
            retryable: true,
            timeout_ms: None,
            side_effects: false,
            subgraph: None,
        },
        EvalFixtureStage::ArtifactCapture => DagNodeExecutionHints {
            parallelizable: false,
            retryable: true,
            timeout_ms: Some(10_000),
            side_effects: true,
            subgraph: None,
        },
    }
}

pub(in super::super::super) fn describe_eval_fixture_graph(
    repro_validate: bool,
) -> DagGraphContract {
    let nodes = build_stage_specs(repro_validate)
        .into_iter()
        .map(|spec| match spec.id {
            EvalFixtureStage::Review => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Run the review pipeline over fixture diff content and collect raw comments."
                        .to_string(),
                kind: DagNodeKind::Execution,
                dependencies: vec![],
                inputs: vec![
                    "config".to_string(),
                    "prepared_fixture".to_string(),
                    "repo_path".to_string(),
                ],
                outputs: vec![
                    "comments".to_string(),
                    "warnings".to_string(),
                    "verification_report".to_string(),
                    "agent_activity".to_string(),
                ],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            EvalFixtureStage::ExpectationMatching => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Match emitted comments against expected and negative fixture findings."
                        .to_string(),
                kind: DagNodeKind::Validation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec!["comments".to_string(), "fixture_expectations".to_string()],
                outputs: vec!["match_summary".to_string(), "failures".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            EvalFixtureStage::CommentCountValidation => DagNodeContract {
                name: spec.id.name().to_string(),
                description: "Check fixture-level minimum and maximum comment count expectations."
                    .to_string(),
                kind: DagNodeKind::Validation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec![
                    "total_comments".to_string(),
                    "fixture_expectations".to_string(),
                    "failures".to_string(),
                ],
                outputs: vec!["failures".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            EvalFixtureStage::BenchmarkMetrics => DagNodeContract {
                name: spec.id.name().to_string(),
                description: "Build benchmark metrics and pass/fail signals from match outcomes."
                    .to_string(),
                kind: DagNodeKind::Analysis,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec![
                    "prepared_fixture".to_string(),
                    "total_comments".to_string(),
                    "match_summary".to_string(),
                    "failures".to_string(),
                ],
                outputs: vec!["benchmark_metrics".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            EvalFixtureStage::ReproductionValidation => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Use bounded tool-backed reproduction checks to validate selected comments."
                        .to_string(),
                kind: DagNodeKind::Validation,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec![
                    "config".to_string(),
                    "prepared_fixture".to_string(),
                    "comments".to_string(),
                ],
                outputs: vec!["reproduction_summary".to_string(), "warnings".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
            EvalFixtureStage::ArtifactCapture => DagNodeContract {
                name: spec.id.name().to_string(),
                description:
                    "Persist fixture-level artifacts for debugging and offline inspection."
                        .to_string(),
                kind: DagNodeKind::Persistence,
                dependencies: spec
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.name().to_string())
                    .collect(),
                inputs: vec![
                    "prepared_fixture".to_string(),
                    "comments".to_string(),
                    "warnings".to_string(),
                    "failures".to_string(),
                    "benchmark_metrics".to_string(),
                ],
                outputs: vec!["artifact_path".to_string()],
                hints: stage_hints(spec.id),
                enabled: spec.enabled,
            },
        })
        .collect::<Vec<_>>();

    DagGraphContract {
        name: "eval_fixture_execution".to_string(),
        description:
            "Fixture-scoped evaluation DAG for review, matching, scoring, reproduction, and artifact capture."
                .to_string(),
        entry_nodes: vec!["review".to_string()],
        terminal_nodes: vec!["artifact_capture".to_string()],
        nodes,
    }
}

fn build_stage_specs(repro_validate: bool) -> Vec<DagNodeSpec<EvalFixtureStage>> {
    vec![
        DagNodeSpec {
            id: EvalFixtureStage::Review,
            dependencies: vec![],
            hints: stage_hints(EvalFixtureStage::Review),
            enabled: true,
        },
        DagNodeSpec {
            id: EvalFixtureStage::ExpectationMatching,
            dependencies: vec![EvalFixtureStage::Review],
            hints: stage_hints(EvalFixtureStage::ExpectationMatching),
            enabled: true,
        },
        DagNodeSpec {
            id: EvalFixtureStage::CommentCountValidation,
            dependencies: vec![EvalFixtureStage::ExpectationMatching],
            hints: stage_hints(EvalFixtureStage::CommentCountValidation),
            enabled: true,
        },
        DagNodeSpec {
            id: EvalFixtureStage::BenchmarkMetrics,
            dependencies: vec![EvalFixtureStage::CommentCountValidation],
            hints: stage_hints(EvalFixtureStage::BenchmarkMetrics),
            enabled: true,
        },
        DagNodeSpec {
            id: EvalFixtureStage::ReproductionValidation,
            dependencies: vec![EvalFixtureStage::Review],
            hints: stage_hints(EvalFixtureStage::ReproductionValidation),
            enabled: repro_validate,
        },
        DagNodeSpec {
            id: EvalFixtureStage::ArtifactCapture,
            dependencies: if repro_validate {
                vec![
                    EvalFixtureStage::BenchmarkMetrics,
                    EvalFixtureStage::ReproductionValidation,
                ]
            } else {
                vec![EvalFixtureStage::BenchmarkMetrics]
            },
            hints: stage_hints(EvalFixtureStage::ArtifactCapture),
            enabled: true,
        },
    ]
}

fn spawn_stage(
    stage: EvalFixtureStage,
    config: &config::Config,
    context: &mut EvalFixtureDagContext,
) -> Result<BoxFuture<'static, Result<EvalFixtureStageOutput>>> {
    match stage {
        EvalFixtureStage::Review => {
            let diff_content = context.prepared.diff_content.clone();
            let repo_path = context.prepared.repo_path.clone();
            let config = config.clone();
            Ok(async move {
                let generation_role = config.generation_model_role.as_str().to_string();
                let generation_provider =
                    config.inferred_provider_label_for_role(config.generation_model_role);
                let generation_model = config.generation_model_name().to_string();
                let review_result =
                    review_diff_content_raw(&diff_content, config, &repo_path).await?;
                let cost_breakdowns = crate::server::cost::review_cost_breakdowns(
                    crate::server::cost::CostBreakdownRequest {
                        workload: "eval_generation",
                        role: &generation_role,
                        provider: generation_provider,
                        model: &generation_model,
                        prompt_tokens: review_result.total_prompt_tokens,
                        completion_tokens: review_result.total_completion_tokens,
                        total_tokens: review_result.total_tokens,
                    },
                    "eval_verification",
                    review_result.verification_report.as_ref(),
                );
                Ok(EvalFixtureStageOutput::Review {
                    cost_breakdowns,
                    verification_report: convert_verification_report(
                        review_result.verification_report,
                    ),
                    agent_activity: convert_agent_activity(review_result.agent_activity),
                    dag_traces: review_result.dag_traces,
                    comments: review_result.comments,
                    warnings: review_result.warnings,
                })
            }
            .boxed())
        }
        EvalFixtureStage::ExpectationMatching => {
            let expectations = context.prepared.fixture.expect.clone();
            let comments = context.comments.clone();
            Ok(async move {
                let match_summary = evaluate_fixture_expectations(&expectations, &comments);
                Ok(EvalFixtureStageOutput::ExpectationMatching {
                    failures: match_summary.failures.clone(),
                    match_summary,
                })
            }
            .boxed())
        }
        EvalFixtureStage::CommentCountValidation => {
            let Some(_) = context.match_summary.as_ref() else {
                anyhow::bail!("comment count validation requires expectation matches");
            };
            let review_summary = core::CommentSynthesizer::generate_summary(&context.comments);
            let total_comments = context.total_comments;
            let expectations = context.prepared.fixture.expect.clone();
            let mut failures = context.failures.clone();
            Ok(async move {
                append_total_comment_failures(&mut failures, total_comments, &expectations);
                append_review_summary_failures(&mut failures, &review_summary, &expectations);
                Ok(EvalFixtureStageOutput::CommentCountValidation { failures })
            }
            .boxed())
        }
        EvalFixtureStage::BenchmarkMetrics => {
            let Some(match_summary) = context.match_summary.clone() else {
                anyhow::bail!("benchmark metrics require expectation matches");
            };
            let prepared = context.prepared.clone();
            let total_comments = context.total_comments;
            let failures = context.failures.clone();
            Ok(async move {
                Ok(EvalFixtureStageOutput::BenchmarkMetrics {
                    benchmark_metrics: build_benchmark_metrics(
                        &prepared,
                        total_comments,
                        &match_summary,
                        &failures,
                    ),
                })
            }
            .boxed())
        }
        EvalFixtureStage::ReproductionValidation => {
            let config = config.clone();
            let prepared = context.prepared.clone();
            let comments = context.comments.clone();
            let repro_max_comments = context.dag_config.repro_max_comments;
            Ok(async move {
                let reproduction_summary = maybe_run_reproduction_validation(
                    &config,
                    &prepared,
                    &comments,
                    repro_max_comments,
                )
                .await?;
                let warnings = reproduction_summary
                    .as_ref()
                    .map(build_reproduction_warnings)
                    .unwrap_or_default();
                let cost_breakdowns = reproduction_summary
                    .as_ref()
                    .and_then(|summary| {
                        (summary.total_tokens > 0).then(|| {
                            crate::server::cost::CostBreakdownRow::new(
                                "eval_auditing",
                                summary.role.as_str(),
                                summary.provider.clone(),
                                summary.model.as_str(),
                                summary.prompt_tokens,
                                summary.completion_tokens,
                                summary.total_tokens,
                            )
                        })
                    })
                    .into_iter()
                    .collect();
                Ok(EvalFixtureStageOutput::ReproductionValidation {
                    reproduction_summary,
                    warnings,
                    cost_breakdowns,
                })
            }
            .boxed())
        }
        EvalFixtureStage::ArtifactCapture => {
            let Some(match_summary) = context.match_summary.clone() else {
                anyhow::bail!("artifact stage requires expectation matching output");
            };
            let prepared = context.prepared.clone();
            let artifact_context = context.dag_config.artifact_context.clone();
            let total_comments = context.total_comments;
            let comments = context.comments.clone();
            let warnings = context.warnings.clone();
            let failures = context.failures.clone();
            let benchmark_metrics = context.benchmark_metrics.clone();
            let verification_report = context.verification_report.clone();
            let agent_activity = context.agent_activity.clone();
            let reproduction_summary = context.reproduction_summary.clone();
            let dag_traces = context.dag_traces.clone();
            Ok(async move {
                let artifact_path = maybe_write_fixture_artifact(EvalFixtureArtifactInput {
                    context: artifact_context.as_ref(),
                    prepared: &prepared,
                    total_comments,
                    comments: &comments,
                    warnings: &warnings,
                    failures: &failures,
                    benchmark_metrics: benchmark_metrics.as_ref(),
                    rule_metrics: &match_summary.rule_metrics,
                    rule_summary: match_summary.rule_summary,
                    verification_report: verification_report.as_ref(),
                    agent_activity: agent_activity.as_ref(),
                    reproduction_summary: reproduction_summary.as_ref(),
                    dag_traces: &dag_traces,
                })
                .await?;
                Ok(EvalFixtureStageOutput::ArtifactCapture { artifact_path })
            }
            .boxed())
        }
    }
}

fn apply_stage_output(
    stage: EvalFixtureStage,
    output: EvalFixtureStageOutput,
    context: &mut EvalFixtureDagContext,
) -> Result<()> {
    match (stage, output) {
        (
            EvalFixtureStage::Review,
            EvalFixtureStageOutput::Review {
                comments,
                warnings,
                cost_breakdowns,
                verification_report,
                agent_activity,
                dag_traces,
            },
        ) => {
            context.total_comments = comments.len();
            context.comments = comments;
            context.warnings = warnings;
            context.cost_breakdowns = cost_breakdowns;
            context.verification_report = verification_report;
            context.agent_activity = agent_activity;
            context.dag_traces = dag_traces;
            Ok(())
        }
        (
            EvalFixtureStage::ExpectationMatching,
            EvalFixtureStageOutput::ExpectationMatching {
                match_summary,
                failures,
            },
        ) => {
            context.match_summary = Some(match_summary);
            context.failures = failures;
            Ok(())
        }
        (
            EvalFixtureStage::CommentCountValidation,
            EvalFixtureStageOutput::CommentCountValidation { failures },
        ) => {
            context.failures = failures;
            Ok(())
        }
        (
            EvalFixtureStage::BenchmarkMetrics,
            EvalFixtureStageOutput::BenchmarkMetrics { benchmark_metrics },
        ) => {
            context.benchmark_metrics = benchmark_metrics;
            Ok(())
        }
        (
            EvalFixtureStage::ReproductionValidation,
            EvalFixtureStageOutput::ReproductionValidation {
                reproduction_summary,
                warnings,
                cost_breakdowns,
            },
        ) => {
            context.reproduction_summary = reproduction_summary;
            context.warnings.extend(warnings);
            context.cost_breakdowns.extend(cost_breakdowns);
            Ok(())
        }
        (
            EvalFixtureStage::ArtifactCapture,
            EvalFixtureStageOutput::ArtifactCapture { artifact_path },
        ) => {
            context.artifact_path = artifact_path;
            Ok(())
        }
        (stage, output) => anyhow::bail!(
            "fixture DAG stage '{}' received incompatible output: {:?}",
            stage.name(),
            output
        ),
    }
}

fn build_reproduction_warnings(summary: &EvalReproductionSummary) -> Vec<String> {
    summary
        .checks
        .iter()
        .filter_map(|check| {
            check.warning.as_ref().map(|warning| {
                format!(
                    "reproduction validator for comment {} ({}) reported: {}",
                    check.comment_id, check.model, warning
                )
            })
        })
        .collect()
}

async fn rewrite_fixture_artifact_with_eval_trace(
    context: &mut EvalFixtureDagContext,
    eval_records: &[DagExecutionRecord],
) -> Result<()> {
    if context.artifact_path.is_none() {
        return Ok(());
    }
    let Some(match_summary) = context.match_summary.as_ref() else {
        anyhow::bail!("artifact rewrite requires expectation matching output");
    };

    let mut dag_traces = context.dag_traces.clone();
    dag_traces.push(DagExecutionTrace {
        graph_name: "eval_fixture_execution".to_string(),
        records: eval_records.to_vec(),
    });
    context.artifact_path = maybe_write_fixture_artifact(EvalFixtureArtifactInput {
        context: context.dag_config.artifact_context.as_ref(),
        prepared: &context.prepared,
        total_comments: context.total_comments,
        comments: &context.comments,
        warnings: &context.warnings,
        failures: &context.failures,
        benchmark_metrics: context.benchmark_metrics.as_ref(),
        rule_metrics: &match_summary.rule_metrics,
        rule_summary: match_summary.rule_summary,
        verification_report: context.verification_report.as_ref(),
        agent_activity: context.agent_activity.as_ref(),
        reproduction_summary: context.reproduction_summary.as_ref(),
        dag_traces: &dag_traces,
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_stage_specs_links_artifact_to_reproduction_when_enabled() {
        let specs = build_stage_specs(true);
        let artifact = describe_dag(&specs)
            .into_iter()
            .find(|spec| spec.name == "artifact_capture")
            .unwrap();

        assert!(artifact
            .dependencies
            .contains(&"benchmark_metrics".to_string()));
        assert!(artifact
            .dependencies
            .contains(&"reproduction_validation".to_string()));
    }

    #[test]
    fn build_stage_specs_keeps_reproduction_optional() {
        let specs = build_stage_specs(false);
        let descriptions = describe_dag(&specs);
        let reproduction = descriptions
            .iter()
            .find(|spec| spec.name == "reproduction_validation")
            .unwrap();
        let artifact = descriptions
            .iter()
            .find(|spec| spec.name == "artifact_capture")
            .unwrap();

        assert!(!reproduction.enabled);
        assert_eq!(artifact.dependencies, vec!["benchmark_metrics"]);
    }

    #[test]
    fn build_stage_specs_marks_reproduction_parallelizable() {
        let specs = build_stage_specs(true);
        let reproduction = specs
            .iter()
            .find(|spec| spec.id == EvalFixtureStage::ReproductionValidation)
            .unwrap();

        assert!(reproduction.hints.parallelizable);
    }

    #[test]
    fn eval_fixture_graph_contract_exposes_reproduction_outputs() {
        let graph = describe_eval_fixture_graph(true);

        assert_eq!(graph.name, "eval_fixture_execution");
        assert_eq!(graph.entry_nodes, vec!["review"]);
        assert!(graph.nodes.iter().any(|node| {
            node.name == "review" && node.hints.subgraph.as_deref() == Some("review_pipeline")
        }));
        assert!(graph.nodes.iter().any(|node| {
            node.name == "reproduction_validation"
                && node.outputs.contains(&"reproduction_summary".to_string())
        }));
    }
}
