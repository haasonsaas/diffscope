# Self-Hosting DiffScope

This guide covers the server deployment path for DiffScope when you want persistent reviews, analytics artifacts, trend history, and operational diagnostics.

## Recommended deployment path

- Use the Helm chart in `charts/diffscope/` for long-running server deployments.
- Use `docker-compose.yml` for local CLI + Ollama workflows only. The compose file runs `diffscope review`, not `diffscope serve`.
- The container image itself defaults to `serve`, so Kubernetes/Helm is the intended self-hosted server path.

## Minimum server checklist

1. Mount a persistent volume at `/home/diffscope/.local/share/diffscope`.
2. Keep the working tree at `/workspace` when you want server-side Git and PR workflows.
3. Set a shared API key with `DIFFSCOPE_SERVER_API_KEY` for protected mutation routes.
4. Point every persisted analytics artifact into the mounted data directory.
5. Schedule `diffscope eval` and `diffscope feedback-eval` if you want trend charts to stay fresh.

## Persistent artifact layout

The server persists and reads these files during normal operation:

| Purpose | Default path | Recommendation for Helm |
| --- | --- | --- |
| Review/event storage | `~/.local/share/diffscope/reviews.json` | keep default |
| Learned conventions | `~/.local/share/diffscope/conventions.json` | keep default |
| Feedback store | `.diffscope.feedback.json` | move under `/home/diffscope/.local/share/diffscope/feedback.json` |
| Eval trend history | `.diffscope.eval-trend.json` | move under `/home/diffscope/.local/share/diffscope/eval-trend.json` |
| Feedback-eval trend history | `.diffscope.feedback-eval-trend.json` | move under `/home/diffscope/.local/share/diffscope/feedback-eval-trend.json` |
| Production replay pack | `~/.local/share/diffscope/eval/production_replay/replay.json` | keep default |
| Failure forensics bundles | `~/.local/share/diffscope/forensics/...` | keep default |

The Helm PVC only covers `/home/diffscope/.local/share/diffscope`, so the relative default paths for `feedback_path`, `eval_trend_path`, and `feedback_eval_trend_path` are not durable unless you override them.

## Example Helm config

```yaml
diffscope:
  model: claude-opus-4-6
  adapter: anthropic
  baseUrl: https://api.anthropic.com

gitRepo:
  enabled: true
  repository: https://github.com/your-org/your-repo.git
  branch: main

persistence:
  enabled: true
  size: 20Gi

config:
  configFile: |
    model: claude-opus-4-6
    model_reasoning: openai/o3
    providers:
      anthropic:
        enabled: true
      openrouter:
        enabled: true
    feedback_path: /home/diffscope/.local/share/diffscope/feedback.json
    eval_trend_path: /home/diffscope/.local/share/diffscope/eval-trend.json
    feedback_eval_trend_path: /home/diffscope/.local/share/diffscope/feedback-eval-trend.json
    retention:
      review_max_age_days: 30
      review_max_count: 1000
      eval_artifact_max_age_days: 30
      trend_history_max_entries: 200

  extraEnv:
    DIFFSCOPE_SERVER_API_KEY: ${DIFFSCOPE_SERVER_API_KEY}
    DIFFSCOPE_GITHUB_AUTO_REVIEW_EVENTS: review_requested
    DIFFSCOPE_GITHUB_REVIEW_REQUEST_REVIEWERS: EvalOpsBot
    ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY}
    OPENROUTER_API_KEY: ${OPENROUTER_API_KEY}
```

Notes:

- `providers.<name>.api_key` and provider-specific environment variables are the recommended way to run mixed-provider installs.
- `openai/o3` and other non-Anthropic `vendor/model` ids route through OpenRouter unless you explicitly force another adapter.
- `config.configFile` is mounted at `/workspace/.diffscope.yml`; it is easiest to use with `gitRepo.enabled: true` so the working directory already matches `/workspace`.

## Secret-management guidance

For enterprise installs, prefer external secret injection over storing credentials in `.diffscope.yml`.

### Recommended secret sources

- `ANTHROPIC_API_KEY`
- `OPENAI_API_KEY`
- `OPENROUTER_API_KEY`
- `DIFFSCOPE_SERVER_API_KEY`
- `DIFFSCOPE_WEBHOOK_SECRET`
- `GITHUB_TOKEN`
- `DIFFSCOPE_GITHUB_APP_ID`
- `DIFFSCOPE_GITHUB_PRIVATE_KEY`
- `DIFFSCOPE_GITHUB_AUTO_REVIEW_EVENTS`
- `DIFFSCOPE_GITHUB_REVIEW_REQUEST_REVIEWERS`
- `DIFFSCOPE_JIRA_BASE_URL`
- `DIFFSCOPE_JIRA_EMAIL`
- `DIFFSCOPE_JIRA_API_TOKEN`
- `DIFFSCOPE_LINEAR_API_KEY`

For an on-demand review bot, set `DIFFSCOPE_GITHUB_AUTO_REVIEW_EVENTS=review_requested`
and `DIFFSCOPE_GITHUB_REVIEW_REQUEST_REVIEWERS=EvalOpsBot`. The GitHub webhook must
subscribe to `pull_request` events; DiffScope will ignore review requests for other
reviewers.

### Validation behavior

`diffscope doctor`, the server doctor endpoint, and startup warnings now surface configuration issues for:

- mixed-provider installs that still rely on legacy top-level `api_key` / `base_url` / `adapter`
- missing provider-specific API keys for selected cloud providers
- incomplete GitHub App config (`github_app_id` + `github_private_key` must be paired)
- incomplete Jira config (`jira_base_url`, `jira_email`, `jira_api_token` must be paired)
- incomplete Vault config (`vault_addr`, `vault_path`, `vault_token`)

### Vault caveat

Vault currently resolves only the legacy top-level `api_key`. For multi-provider installs, use provider-specific secrets from your runtime environment or secret store injection.

## Analytics and retention

### What populates Analytics

- `/api/events` and `/api/events/stats` come from stored reviews and wide events.
- `/api/analytics/learned-rules` reads the convention store.
- `/api/analytics/rejected-patterns` reads the feedback store.
- `/api/analytics/trends` and `/api/analytics/attention-gaps` read the eval and feedback-eval trend JSON files.

### How to keep trend data fresh

Run these on a schedule against a persisted artifact directory:

```bash
diffscope eval --fixtures eval/fixtures --artifact-dir /var/lib/diffscope/eval
diffscope feedback-eval --input /home/diffscope/.local/share/diffscope/reviews.json --eval-report /var/lib/diffscope/eval/report.json
```

### Retention controls

- `retention.review_max_age_days`
- `retention.review_max_count`
- `retention.eval_artifact_max_age_days`
- `retention.trend_history_max_entries`

Completed reviews apply review retention automatically. Eval artifact pruning happens when you run `diffscope eval --artifact-dir ...`.

### Analytics recompute job

Use these protected endpoints after scoring or schema changes:

```text
POST /api/analytics/recompute
GET  /api/analytics/recompute/{job_id}
```

The recompute job refreshes stored review summaries and event aggregates. It does not rebuild eval or feedback-eval trend files.

## Operations and diagnostics

### Health checks

- `diffscope doctor`
- `GET /api/status`
- `GET /api/doctor`
- `GET /metrics`
- Helm `test-connection`

### Failure forensics bundles

DiffScope now writes JSON forensics bundles for degraded review and eval runs.

- Review bundles: `~/.local/share/diffscope/forensics/reviews/<review-id>/`
- Eval bundles: `~/.local/share/diffscope/forensics/eval/...` or `<artifact-dir>/forensics/...`

Review manifests are available from the protected endpoint:

```text
GET /api/review/{id}/forensics
```

### Production replay evals

Accepted and rejected review outcomes are captured into an anonymized replay pack at:

```text
~/.local/share/diffscope/eval/production_replay/replay.json
```

Run it with the normal eval command:

```bash
diffscope eval --fixtures ~/.local/share/diffscope/eval/production_replay
```
