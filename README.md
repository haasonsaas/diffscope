# DiffScope

A composable code review engine for automated diff analysis.

## Features

- **Model Agnostic**: Works with OpenAI, Anthropic Claude, Ollama, and any OpenAI-compatible API
- **Git Integration**: Review uncommitted, staged, or branch changes directly
- **PR Reviews**: Analyze and comment on GitHub pull requests with interactive commands
- **Smart Prompting**: Advanced prompt engineering with examples, XML structure, and chain-of-thought
- **Commit Messages**: AI-powered commit message suggestions following conventional commits
- **Composable Architecture**: Modular components that work together
- **Plugin System**: Extensible pre-analyzers and post-processors
- **Multiple Outputs**: JSON, patch, or markdown formats
- **CI/CD Ready**: GitHub Action, GitLab CI, and Docker support
- **Smart Review**: Enhanced analysis with confidence scoring, fix effort estimation, and executive summaries
- **Path-Based Configuration**: Customize review behavior for different parts of your codebase
- **Signal Controls**: Tune strictness and comment types (`logic`, `syntax`, `style`, `informational`)
- **Adaptive Learning**: Suppress low-value recurring feedback based on accepted/rejected review history
- **Scoped Custom Context**: Attach rules and reference files to path scopes for higher-precision reviews
- **Pattern Repositories**: Pull review context from shared cross-repo rule libraries
- **Comment Follow-Ups**: Ask threaded questions on generated review comments with `diffscope discuss`
- **Changelog Generation**: Generate changelogs and release notes from git history
- **Interactive Commands**: Respond to PR comments with @diffscope commands

## Quick Start

### Install Pre-built Binary (Recommended)

#### Linux/macOS
```bash
curl -sSL https://raw.githubusercontent.com/evalops/diffscope/main/install.sh | sh
```

#### Windows (PowerShell)
```powershell
iwr -useb https://raw.githubusercontent.com/evalops/diffscope/main/install.ps1 | iex
```

#### Manual Download
Download the latest binary for your platform from the [releases page](https://github.com/evalops/diffscope/releases/latest):
- Linux: `diffscope-x86_64-unknown-linux-musl`
- macOS Intel: `diffscope-x86_64-apple-darwin`
- macOS Apple Silicon: `diffscope-aarch64-apple-darwin`
- Windows: `diffscope-x86_64-pc-windows-msvc.exe`

### Install via Package Managers

#### Homebrew (macOS/Linux)
```bash
brew tap evalops/diffscope
brew install diffscope
```

#### Cargo (requires Rust)
```bash
cargo install diffscope
```

### Docker
```bash
# Pull the latest image
docker pull ghcr.io/evalops/diffscope:latest

# Run with current directory mounted
docker run --rm -v $(pwd):/workspace ghcr.io/evalops/diffscope:latest review --diff /workspace/pr.diff

# Create an alias for convenience
alias diffscope='docker run --rm -v $(pwd):/workspace ghcr.io/evalops/diffscope:latest'
```

## Usage

### Basic Usage
```bash
# Review your current changes
git diff | diffscope review

# Review a specific file diff
diffscope review --diff patch.diff

# Run without stdin to review uncommitted changes
diffscope review

# Get enhanced analysis with smart review
git diff | diffscope smart-review

# Evaluate reviewer quality against fixtures
diffscope eval --fixtures eval/fixtures --output eval-report.json
```

### Git Integration
```bash
# Review what you're about to commit
diffscope git staged

# Review all uncommitted changes  
diffscope git uncommitted

# Compare your branch to main
diffscope git branch main

# Compare your branch to the repo default
diffscope git branch

# Get AI-powered commit message suggestions
diffscope git suggest
```

### Pull Request Review
```bash
# Review the current PR
diffscope pr

# Review a specific PR number
diffscope pr --number 123

# Post review comments directly to GitHub
diffscope pr --post-comments
```

`--post-comments` now attempts inline file/line comments first, falls back to PR-level comments when GitHub rejects an anchor, and upserts a sticky DiffScope summary comment on the PR.
When rule matching is active, DiffScope also includes detected `rule_id` values in PR comments and summaries.

### Evaluation Fixtures
```yaml
name: auth guard regression
repo_path: ../../
diff_file: ./auth.patch
expect:
  must_find:
    - file: src/api/auth.rs
      line: 42
      contains: missing auth check
      severity: error
      category: security
      rule_id: sec.auth.guard      # label for per-rule precision/recall
      require_rule_id: false       # set true to require model-emitted RULE id
  must_not_find:
    - contains: style
  min_total: 1
  max_total: 8
```

`diffscope eval` now reports per-rule precision/recall/F1 (micro and macro), and includes top rule-level TP/FP/FN counts in CLI and JSON output.
Starter fixtures live in `eval/fixtures/repo_regressions`.
Markdown and smart-review reports now include rule-level issue breakdown tables when rule ids are available.

Threshold flags for CI gates:
```bash
diffscope eval \
  --fixtures eval/fixtures \
  --output eval-report.json \
  --baseline eval-baseline.json \
  --max-micro-f1-drop 0.15 \
  --min-micro-f1 0.30 \
  --min-rule-f1 sec.shell.injection=0.20 \
  --max-rule-f1-drop sec.shell.injection=0.15
```

### Smart Review (Enhanced Analysis)
```bash
# Get professional-grade analysis with confidence scoring
git diff | diffscope smart-review

# Generate executive summary report
diffscope smart-review --diff changes.patch --output report.md

# Review with specific AI model
git diff | diffscope smart-review --model claude-3-5-sonnet-20241022
```

### AI Model Configuration
```bash
# OpenAI (default)
export OPENAI_API_KEY=your-key
git diff | diffscope review --model gpt-4o

# Force OpenAI Responses API usage
git diff | diffscope review --openai-responses true

# Anthropic Claude
export ANTHROPIC_API_KEY=your-key  
git diff | diffscope review --model claude-3-5-sonnet-20241022

# Local Ollama
git diff | diffscope review --model ollama:codellama

# Custom API endpoint
export OPENAI_BASE_URL=https://api.custom.com/v1
git diff | diffscope review --model custom-model
```

### Self-Hosted / Local Models

Run DiffScope against a local LLM with zero cloud dependencies. No API key required.
For the server deployment path with persistent analytics, retention, secret-management guidance, and forensics bundles, see [`docs/self-hosting.md`](docs/self-hosting.md).

#### Ollama (Recommended)
```bash
# Install Ollama and pull a code model
ollama pull codellama

# Review code with local model
git diff | diffscope review --base-url http://localhost:11434 --model ollama:codellama

# Or use a config file (see examples/selfhosted-ollama.yml)
cp examples/selfhosted-ollama.yml .diffscope.yml
git diff | diffscope review
```

#### vLLM / LM Studio / OpenAI-Compatible Servers
```bash
# Point to any OpenAI-compatible endpoint
git diff | diffscope review \
  --base-url http://localhost:8000/v1 \
  --adapter openai \
  --model deepseek-coder-6.7b

# See examples/selfhosted-vllm.yml for a ready-made config
```

#### Docker Compose (Ollama + DiffScope)
```bash
# Start Ollama (with GPU) and DiffScope together — model is auto-pulled
docker compose up diffscope-local

# CPU-only mode (no NVIDIA GPU required)
docker compose --profile cpu up ollama-cpu

# Pull a specific model manually
docker compose exec ollama ollama pull deepseek-coder:6.7b-instruct
```

#### Docker with GPU

Requires [NVIDIA Container Toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html):

```bash
# Install NVIDIA Container Toolkit (Ubuntu/Debian)
curl -fsSL https://nvidia.github.io/libnvidia-container/gpgkey | sudo gpg --dearmor -o /usr/share/keyrings/nvidia-container-toolkit-keyring.gpg
curl -s -L https://nvidia.github.io/libnvidia-container/stable/deb/nvidia-container-toolkit.list | \
  sed 's#deb https://#deb [signed-by=/usr/share/keyrings/nvidia-container-toolkit-keyring.gpg] https://#g' | \
  sudo tee /etc/apt/sources.list.d/nvidia-container-toolkit.list
sudo apt-get update && sudo apt-get install -y nvidia-container-toolkit
sudo nvidia-ctk runtime configure --runtime=docker && sudo systemctl restart docker

# Run with GPU
docker compose up diffscope-local
```

If you don't have an NVIDIA GPU, use the CPU-only profile or run Ollama directly on the host.

#### Recommended Models

| RAM Available | Recommended Model | Command |
|---|---|---|
| 8 GB | Codellama 7B Q4 | `ollama pull codellama:7b-instruct-q4_0` |
| 16 GB | Deepseek Coder 6.7B | `ollama pull deepseek-coder:6.7b-instruct` |
| 16 GB | Codellama 13B Q4 | `ollama pull codellama:13b-instruct-q4_0` |
| 32+ GB | Deepseek Coder 33B Q4 | `ollama pull deepseek-coder:33b-instruct-q4_0` |

For best results, use instruction-tuned (`-instruct`) variants of code-specialized models.

#### Performance Tuning

For local models, adjust these config values based on your model's context window:

```yaml
# .diffscope.yml for 7B model on 16GB RAM
context_window: 8192        # Model's actual context limit
max_tokens: 2048            # Max response length
max_diff_chars: 12000       # Truncate large diffs
max_context_chars: 8000     # Limit surrounding code context
context_max_chunks: 8       # Max context files to include
temperature: 0.1            # Low temp for consistent reviews
```

**Tips:**
- `diffscope doctor` shows detected context window and tests inference speed
- Quantized models (Q4, Q5) use ~50-60% less RAM with minimal quality loss
- GPU inference is 5-10x faster than CPU-only
- First request after model load is slower (loading into VRAM)

#### Check Your Setup
```bash
# Verify endpoint reachability, models, and recommendations
diffscope doctor
diffscope doctor --base-url http://localhost:11434
```

#### Troubleshooting

**Model is slow (>30 seconds per review)**
- Check tokens/sec with `diffscope doctor`
- Try a quantized model: `ollama pull codellama:7b-instruct-q4_0`
- Reduce context: set `max_diff_chars: 8000` and `context_window: 4096`

**Out of memory errors**
- Use a smaller model (7B instead of 13B)
- Use heavier quantization (Q4 instead of Q8)
- Set `context_window` lower in config (e.g., 4096)
- Monitor with `nvidia-smi` (GPU) or `htop` (RAM)

**Empty or garbage reviews**
- Run `diffscope doctor` to test model responsiveness
- Try a code-specialized model (deepseek-coder, codellama)
- Avoid models smaller than 3B for code review

**"Endpoint unreachable" error**
- Verify server is running: `curl http://localhost:11434/api/tags`
- Check the port matches your `base_url` config
- For Docker: ensure services are on the same network

#### Environment Variables
| Variable | Description |
|----------|-------------|
| `DIFFSCOPE_BASE_URL` | LLM API base URL (also accepts `OPENAI_BASE_URL`) |
| `DIFFSCOPE_API_KEY` | API key for the LLM endpoint |
| `DIFFSCOPE_GITHUB_AUTO_REVIEW_EVENTS` | Comma-separated `pull_request` actions that start webhook reviews, such as `opened,synchronize` or `review_requested` |
| `DIFFSCOPE_GITHUB_REVIEW_REQUEST_REVIEWERS` | Comma-separated GitHub logins whose requested reviews trigger `review_requested` automation, for example `EvalOpsBot` |

#### CLI Flags
| Flag | Description |
|------|-------------|
| `--base-url` | LLM API base URL |
| `--api-key` | API key (optional for local servers) |
| `--adapter` | Force adapter: `openai`, `anthropic`, or `ollama` |

### Supported Models

**OpenAI**: gpt-4o, gpt-4-turbo, gpt-3.5-turbo

**Anthropic**: claude-3-5-sonnet-20241022, claude-3-5-haiku-20240307, claude-3-opus-20240229, claude-3-haiku-20240307, and newer Claude models

**Ollama**: Any locally installed model (codellama, llama3.2, mistral, etc.) - use `ollama:model-name` format

### Output Formats
```bash
# JSON output (default)
git diff | diffscope review --output-format json

# Markdown report  
git diff | diffscope review --output-format markdown > review.md

# Inline patch comments
git diff | diffscope review --output-format patch

# Follow-up Q&A on generated comments
diffscope discuss --review review.json --comment-index 1 --question "Is this still an issue if we add caching?"
```

## GitHub Action

```yaml
name: AI Code Review
on: [pull_request]

jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: evalops/diffscope@v1
        with:
          model: gpt-4o
          openai-api-key: ${{ secrets.OPENAI_API_KEY }}
          post-comments: true
```

## Configuration

Create a `.diffscope.yml` file in your repository:

```yaml
model: gpt-4o
temperature: 0.2
max_tokens: 4000
max_context_chars: 20000  # 0 disables context truncation
max_diff_chars: 40000     # 0 disables diff truncation
context_max_chunks: 24     # Max context chunks sent to the model per file
context_budget_chars: 24000 # Hard cap for ranked context payload per file
min_confidence: 0.0       # Drop comments below this confidence (0.0-1.0)
strictness: 2             # 1 = high-signal only, 2 = balanced, 3 = deep scan
comment_types:
  - logic
  - syntax
  - style
  - informational
review_profile: balanced  # balanced | chill | assertive
review_instructions: |
  Prioritize security and correctness issues. Avoid stylistic comments unless they impact maintainability.
smart_review_summary: true   # Include AI-generated PR summary in smart-review output
smart_review_diagram: false  # Generate a Mermaid diagram in smart-review output
symbol_index: true           # Build repo symbol index for cross-file context (respects .gitignore)
symbol_index_provider: regex # regex | lsp
symbol_index_lsp_command: rust-analyzer
symbol_index_lsp_languages:
  rs: rust
symbol_index_max_files: 500
symbol_index_max_bytes: 200000
symbol_index_max_locations: 5
symbol_index_graph_hops: 2
symbol_index_graph_max_files: 12
feedback_path: ".diffscope.feedback.json"
system_prompt: "Focus on security vulnerabilities, performance issues, and best practices"
openai_use_responses: true  # Use OpenAI Responses API (recommended) instead of chat completions

custom_context:
  - scope: "src/api/**"
    notes:
      - "Auth flows must enforce tenant boundaries and rate limits."
    files:
      - "docs/security/*.md"
      - "src/config/**/*.yml"

pattern_repositories:
  - source: "../shared-review-patterns" # local path or git URL
    scope: "src/**"
    include_patterns:
      - "rules/**/*.md"
      - "examples/**/*.yml"
    max_files: 8
    max_lines: 200
    rule_patterns:
      - "policy/**/*.yml"
      - "policy/**/*.json"
    max_rules: 200

# Repository-level rule files (YAML/JSON)
rules_files:
  - ".diffscope-rules.yml"
  - "rules/**/*.yml"
max_active_rules: 30
rule_priority:
  - "sec.shell.injection"
  - "sec.auth.guard"
  - "reliability.unwrap_panic"

# Built-in plugins (enabled by default)
plugins:
  eslint: true          # JavaScript/TypeScript linting
  semgrep: true         # Security-focused static analysis  
  duplicate_filter: true # Remove duplicate comments

# Global exclusions
exclude_patterns:
  - "**/*.generated.*"
  - "**/node_modules/**"
  - "**/__pycache__/**"
```

Set `symbol_index_provider: lsp` to use a language server; it falls back to regex indexing if the LSP binary is missing. Configure `symbol_index_lsp_languages` and `symbol_index_lsp_command` to match your server (for example, `typescript-language-server --stdio` with `ts`/`tsx` language IDs). If you omit `symbol_index_lsp_command`, diffscope will try to auto-detect a server based on installed binaries and the file types in your repo. You can also force a server for a single run with `--lsp-command`.

### LSP Symbol Index Examples (All Common Languages)

Pick one LSP server per run (one `symbol_index_lsp_command`). Update the language map to match the server you installed.

```yaml
# Rust (rust-analyzer)
symbol_index_provider: lsp
symbol_index_lsp_command: rust-analyzer
symbol_index_lsp_languages:
  rs: rust

# TypeScript / JavaScript (typescript-language-server)
# symbol_index_provider: lsp
# symbol_index_lsp_command: "typescript-language-server --stdio"
# symbol_index_lsp_languages:
#   ts: typescript
#   tsx: typescriptreact
#   js: javascript
#   jsx: javascriptreact

# Python (python-lsp-server / pylsp)
# symbol_index_provider: lsp
# symbol_index_lsp_command: pylsp
# symbol_index_lsp_languages:
#   py: python
#   pyi: python

# Go (gopls)
# symbol_index_provider: lsp
# symbol_index_lsp_command: gopls
# symbol_index_lsp_languages:
#   go: go

# Java (Eclipse JDT LS)
# symbol_index_provider: lsp
# symbol_index_lsp_command: "jdtls -configuration /path/to/config -data /path/to/workspace"
# symbol_index_lsp_languages:
#   java: java

# Kotlin (Kotlin LSP)
# symbol_index_provider: lsp
# symbol_index_lsp_command: kotlin-lsp
# symbol_index_lsp_languages:
#   kt: kotlin

# C / C++ (clangd)
# symbol_index_provider: lsp
# symbol_index_lsp_command: clangd
# symbol_index_lsp_languages:
#   c: c
#   h: c
#   cpp: cpp
#   hpp: cpp

# C# (csharp-ls)
# symbol_index_provider: lsp
# symbol_index_lsp_command: csharp-ls
# symbol_index_lsp_languages:
#   cs: csharp

# Ruby (solargraph)
# symbol_index_provider: lsp
# symbol_index_lsp_command: "solargraph stdio"
# symbol_index_lsp_languages:
#   rb: ruby

# PHP (Phpactor)
# symbol_index_provider: lsp
# symbol_index_lsp_command: "phpactor language-server"
# symbol_index_lsp_languages:
#   php: php
```

### LSP Setup Notes (Install + Command)

For detailed install commands, OS-specific package manager options, and troubleshooting, see `docs/lsp.md`. For ready-made configs per language, see `examples/lsp/`.

You can validate your setup with:

```
diffscope lsp-check
```

## Plugin Development

Create custom analyzers:

```typescript
export interface PreAnalyzer {
  id: string
  run(diff: UnifiedDiff, repoPath: string): Promise<LLMContextChunk[]>
}

export interface PostProcessor {
  id: string
  run(comments: Comment[], repoPath: string): Promise<Comment[]>
}
```

## Architecture

```mermaid
graph LR
  A[git diff] --> B(core-engine)
  subgraph core-engine
    B1[Diff Parser]
    B2[Context Fetcher]
    B3[Prompt Builder]
    B4[LLM Adapter]
    B5[Comment Synthesizer]
  end
  B -->|JSON| C(output)
```

## License

Apache-2.0 License. See [LICENSE](LICENSE) for details.

## Example Output

### Standard Review
```json
[
  {
    "file_path": "src/auth.py",
    "line_number": 42,
    "content": "Potential SQL injection vulnerability",
    "severity": "Error",
    "category": "Security",
    "suggestion": "Use parameterized queries instead of string interpolation"
  }
]
```

### Smart Review Output
```markdown
# 🤖 Smart Review Analysis Results

## 📊 Executive Summary

🟡 **Code Quality Score:** 8.2/10
📝 **Total Issues Found:** 4
🚨 **Critical Issues:** 1
📁 **Files Analyzed:** 3

## 🧾 PR Summary

**Add auth safeguards** (Fix)

### Key Changes

- Harden auth query handling
- Add route-level guards
- Introduce safe defaults for user lookups

### Diagram

```mermaid
flowchart TD
  A[Request] --> B[Auth Guard]
  B --> C[DB Query]
  C --> D[Response]
```

## 🧭 Change Walkthrough

- `src/auth.py` (modified; +12, -3)
- `src/models.py` (modified; +8, -1)
- `src/routes.py` (new; +24, -0)

### 🎯 Priority Actions
1. Address 1 security issue(s) immediately
2. Consider performance optimization for database queries

---

## 🔍 Detailed Analysis

### 🔴 Critical Issues (Fix Immediately)

#### 🔒 **src/auth.py:42** - 🔴 Significant Effort Security
**Confidence:** 95% | **Tags:** `security`, `sql`, `injection`

SQL injection vulnerability detected. User input is directly interpolated into query string without proper sanitization.

**💡 Recommended Fix:**
Use parameterized queries to prevent SQL injection attacks.

**🔧 Code Example:**
```diff
- query = f"SELECT * FROM users WHERE username='{username}'"
+ query = "SELECT * FROM users WHERE username=%s"
+ cursor.execute(query, (username,))
```

### 🟡 High Priority Issues

#### ⚡ **src/models.py:28** - 🟡 Moderate Effort Performance
**Confidence:** 87% | **Tags:** `performance`, `n+1-query`

N+1 query problem detected in user retrieval loop.

**💡 Recommended Fix:**
Use eager loading or bulk queries to reduce database calls.
```

### Commit Message Suggestion
```
feat(auth): add JWT-based authentication system
```

## Author

Jonathan Haas <jonathan@haas.holdings>

## Advanced CI/CD Integration

See `.github/workflows/eval.yml` for a ready-to-run quality gate that compares PR eval metrics against `origin/main` and fails on micro-F1 or rule-level regressions.

### Enterprise GitHub Actions Workflow

Here's an example of how large organizations use diffscope in production CI/CD pipelines:

```yaml
name: AI Code Review
on:
  pull_request:
    types: [opened, synchronize]
    branches: [main]

jobs:
  ai-code-review:
    name: AI Code Review with DiffScope
    runs-on: ubuntu-latest
    permissions:
      contents: read
      pull-requests: write

    steps:
      - name: Checkout PR
        uses: actions/checkout@v4
        with:
          fetch-depth: 0
          ref: ${{ github.event.pull_request.head.sha }}

      - name: Install DiffScope with Cache
        uses: actions/cache@v4
        with:
          path: ~/.cargo/bin
          key: ${{ runner.os }}-diffscope-${{ hashFiles('**/Cargo.lock') }}
      
      - run: |
          if ! command -v diffscope &> /dev/null; then
            cargo install diffscope
          fi

      - name: Generate PR Diff
        run: |
          git diff origin/${{ github.event.pull_request.base.ref }}...HEAD > pr.diff

      - name: Run AI Review
        env:
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
        run: |
          diffscope review --model claude-3-5-sonnet-20241022 \
            --diff pr.diff --output-format json > review.json

      - name: Post Review Comments
        uses: actions/github-script@v7
        with:
          script: |
            const fs = require('fs');
            const review = JSON.parse(fs.readFileSync('review.json', 'utf8'));
            
            let body = '## 🤖 AI Code Review\n\n';
            if (review.length === 0) {
              body += '✅ **No issues found!** Code looks good!';
            } else {
              body += review.map((item, i) => 
                `**${i+1}.** \`${item.file_path}:${item.line_number}\`\n` +
                `${item.content}\n` +
                (item.suggestion ? `\n💡 **Suggestion:** ${item.suggestion}\n` : '')
              ).join('\n---\n');
            }
            
            github.rest.issues.createComment({
              issue_number: context.issue.number,
              owner: context.repo.owner,
              repo: context.repo.repo,
              body: body
            });
```

### Enterprise Configuration Example

For a large Python/FastAPI application at a company like Acme Inc:

**.diffscope.yml**
```yaml
# Acme Inc DiffScope Configuration
model: "claude-3-5-sonnet-20241022"
temperature: 0.1  # Low for consistent reviews
max_tokens: 4000
min_confidence: 0.2
review_profile: assertive
smart_review_summary: true
smart_review_diagram: true
symbol_index: true

system_prompt: |
  You are reviewing Python code for a production FastAPI application.
  
  Critical focus areas:
  - SQL injection and security vulnerabilities
  - Async/await correctness
  - Resource leaks and memory issues
  - API contract consistency
  - Production deployment concerns
  
  Prioritize by severity: Security > Performance > Maintainability

# File filters for monorepo
exclude_patterns:
  - "**/__pycache__/**"
  - "**/venv/**"
  - "**/.pytest_cache/**"
  - "**/node_modules/**"
  - "**/*.generated.*"

# Review configuration
max_context_chars: 20000
max_diff_chars: 40000
```

### Integration with Other CI Tools

**GitLab CI Example:**
```yaml
code-review:
  stage: review
  image: rust:alpine
  only:
    - merge_requests
  script:
    - apk add --no-cache git
    - cargo install diffscope
    - git diff origin/$CI_MERGE_REQUEST_TARGET_BRANCH_NAME...HEAD > mr.diff
    - diffscope smart-review --diff mr.diff --output review.md
  artifacts:
    reports:
      codequality: review.md
```

**Jenkins Pipeline:**
```groovy
stage('AI Code Review') {
  steps {
    sh '''
      curl -sSL https://sh.rustup.rs | sh -s -- -y
      source $HOME/.cargo/env
      cargo install diffscope
      
      git diff origin/${env.CHANGE_TARGET}...HEAD > pr.diff
      diffscope review --diff pr.diff --output-format json > review.json
    '''
    
    publishHTML([
      allowMissing: false,
      alwaysLinkToLastBuild: true,
      keepAll: true,
      reportDir: '.',
      reportFiles: 'review.json',
      reportName: 'AI Code Review'
    ])
  }
}
```

### Best Practices for CI/CD Integration

1. **Cache Installation**: Cache cargo/diffscope binaries to speed up CI runs
2. **API Key Management**: Use secure secret storage for API keys
3. **Diff Size Limits**: Set max diff size to avoid timeouts on large PRs
4. **Custom Prompts**: Tailor system prompts to your tech stack and standards
5. **Output Parsing**: Handle both empty reviews and JSON parsing errors gracefully
6. **Conditional Runs**: Skip reviews on draft PRs or specific file types

## Available Commands

### Core Commands
```bash
# Review diffs
diffscope review [--diff file.patch]

# Enhanced analysis with confidence scoring
diffscope smart-review [--diff file.patch]

# Git integration
diffscope git uncommitted    # Review uncommitted changes
diffscope git staged         # Review staged changes
diffscope git branch [base]  # Compare against branch (default: repo default)
diffscope git suggest        # Generate commit messages
diffscope git pr-title       # Generate PR titles

# Pull request operations
diffscope pr [--number N] [--post-comments] [--summary]

# Repository check (uncommitted changes at path)
diffscope check [path]

# LSP preflight checks
diffscope lsp-check [path]

# File comparison
diffscope compare --old-file old.py --new-file new.py

# Changelog generation
diffscope changelog --from v0.4.0 [--to HEAD] [--release v0.5.0]
```

## New Features in v0.5.3

### 🔄 Changelog Generation

Generate professional changelogs and release notes from your git history:

```bash
# Generate changelog from a specific tag to HEAD
diffscope changelog --from v0.4.0 --to HEAD

# Generate release notes for a new version
diffscope changelog --release v0.5.0 --from v0.4.0

# Output to file
diffscope changelog --from v0.4.0 --output CHANGELOG.md
```

The changelog generator:
- Parses conventional commits automatically
- Groups changes by type (features, fixes, etc.)
- Highlights breaking changes
- Shows contributor statistics
- Generates both changelogs and release notes formats

### 🎯 Path-Based Configuration

Customize review behavior for different parts of your codebase:

**.diffscope.yml**
```yaml
# Global configuration
model: gpt-4o
temperature: 0.2
max_tokens: 4000

# Exclude patterns
exclude_patterns:
  - "**/*.generated.*"
  - "**/node_modules/**"

# Path-specific rules
paths:
  # API endpoints need security focus
  "src/api/**":
    ignore_patterns:
      - "**/*.generated.*"
    extra_context:
      - "src/auth/**"
    review_instructions: |
      Prioritize auth, validation, and sensitive data handling.
    system_prompt: |
      Focus on SQL injection, auth bypass, and input validation
    severity_overrides:
      security: error  # All security issues become errors

  # Test files have different standards  
  "tests/**":
    ignore_patterns:
      - "**/*.snap"
    extra_context:
      - "src/main/**"
    severity_overrides:
      style: suggestion  # Style issues are just suggestions

  # Database migrations are critical
  "migrations/**":
    severity_overrides:
      bug: error  # Any bug in migrations is critical
```

### 💬 Interactive PR Commands

*Note: Interactive commands are currently in development.*

Planned support for responding to pull request comments with interactive commands:

```
@diffscope review                 # Re-review the changes
@diffscope review security        # Focus review on security
@diffscope ignore src/generated/  # Ignore specific paths
@diffscope explain line 42        # Explain specific code
@diffscope generate tests         # Generate unit tests
@diffscope help                   # Show all commands
```

### Requested Reviewer Automation

Server webhook deployments can run reviews only when a specific reviewer is requested. For an org-level EvalOpsBot setup, subscribe the GitHub App or webhook to `pull_request` events and run the server with:

```bash
DIFFSCOPE_GITHUB_AUTO_REVIEW_EVENTS=review_requested
DIFFSCOPE_GITHUB_REVIEW_REQUEST_REVIEWERS=EvalOpsBot
```

With that configuration, `pull_request.review_requested` starts a full DiffScope review only when GitHub reports `requested_reviewer.login` as `EvalOpsBot`.

### ✅ Feedback Loop (Reduce Repeated False Positives)

Use the feedback store to suppress comments you’ve already reviewed:

```bash
# Reject comments from a prior JSON review output
diffscope feedback --reject review.json

# Accept comments (keeps a record, and removes them from suppress list)
diffscope feedback --accept review.json
```

The feedback file defaults to `.diffscope.feedback.json` and can be configured in `.diffscope.yml`.

**CI helper (GitHub Actions):**

```yaml
- name: Update DiffScope feedback
  if: always()
  run: |
    bash scripts/update_feedback_from_review.sh \
      --action reject \
      --input review.json \
      --feedback .diffscope.feedback.json
```

### 📊 PR Summary Generation

Generate executive summaries for pull requests:

```bash
# Generate PR summary with statistics
diffscope pr --summary

# Generate and post to GitHub
diffscope pr --number 123 --summary --post-comments
```

The summary includes:
- Change statistics and impact analysis
- Key modifications by category
- Risk assessment
- Review recommendations

## Contributing

Contributions are welcome! Please open an issue first to discuss what you would like to change. Enhancement backlog and triage: see [docs/ROADMAP.md](docs/ROADMAP.md) and `gh issue list --label "priority: high"`.

**PR workflow:** Open a PR → ensure CI is green (version, lint, security, test, mutation, review) → merge when ready. Use a short test plan in the PR description. Small, focused PRs are preferred. Use the PR template (Summary, Test plan, **Closes #N**). When the PR is merged, a workflow will comment on each linked issue. **Release process:** [docs/release-process.md](docs/release-process.md) (version bump, RELEASE_NOTES, Prepare release workflow). **gh CLI:** [docs/gh-automation.md](docs/gh-automation.md) (issues, PRs, workflow run, release from terminal).

### Local Development Checks

Enable the repository-managed git hooks after cloning:

```bash
bash scripts/install-hooks.sh
```

**Pre-commit** runs only when relevant files are staged:
- Rejects merge conflict markers and requires text files to end with exactly one newline
- Validates GitHub Actions workflows (install `actionlint` for full workflow linting)
- When Rust files are staged: `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test`
- When `web/` is staged: `npm run lint`, `tsc -b`, and `npm run test` in `web/`
- When `scripts/` or `.githooks/` change: `shellcheck` (if installed)

**Pre-push** runs the full gate: workflow check, version sync (`Cargo.toml` vs git tags), `cargo fmt`, `cargo clippy`, `cargo audit` (if installed), `npm ci && npm run build && npm run test` in `web/`, and `cargo test`. The first push after clone may take longer due to `npm ci`. Mutation testing runs in CI only (see `docs/mutation-testing.md`), not on pre-push, to keep pushes fast. For a quick push without full checks use `git push --no-verify` (use sparingly).

## Supported Platforms

DiffScope provides pre-built binaries for the following platforms:

| Platform | Architecture | Binary |
|----------|-------------|---------|
| Linux | x86_64 | `diffscope-x86_64-unknown-linux-musl` (static, works on all distros) |
| Linux | x86_64 | `diffscope-x86_64-unknown-linux-gnu` |
| Linux | ARM64 | `diffscope-aarch64-unknown-linux-gnu` |
| macOS | Intel (x86_64) | `diffscope-x86_64-apple-darwin` |
| macOS | Apple Silicon (ARM64) | `diffscope-aarch64-apple-darwin` |
| Windows | x86_64 | `diffscope-x86_64-pc-windows-msvc.exe` |

All binaries are automatically built and uploaded with each release.

## Support

- GitHub Issues: [github.com/evalops/diffscope/issues](https://github.com/evalops/diffscope/issues)
