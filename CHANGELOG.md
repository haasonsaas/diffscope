# Changelog

All notable changes to this project will be documented in this file.

## [0.4.2] - 2025-06-06

### Added
- **Advanced CI/CD Integration**: Comprehensive enterprise examples section
- **GitHub Actions**: Full production workflow with caching and comment posting
- **GitLab CI**: Integration example for GitLab pipelines
- **Jenkins**: Pipeline script for Jenkins integration
- **Enterprise Config**: Real-world .diffscope.yml for large Python/FastAPI projects
- **Best Practices**: CI/CD integration tips and recommendations

### Documentation
- Added "Advanced CI/CD Integration" section with production examples
- Enterprise configuration patterns for monorepos
- Multi-platform CI examples (GitHub, GitLab, Jenkins)
- Best practices for API key management and caching

## [0.4.1] - 2025-06-06

### Changed
- **Documentation**: Cleaned up README with more practical and realistic examples
- **Installation**: Updated examples to use `cargo install diffscope` from crates.io
- **Usage Examples**: Simplified and focused on common developer workflows
- **Output Examples**: More realistic security vulnerability demonstrations
- **Configuration**: Streamlined examples for better clarity

## [0.4.0] - 2025-06-06

### Added
- **Smart Review System**: New `smart-review` command with enhanced analysis capabilities
- **Confidence Scoring**: Each issue now includes confidence percentage (0-100%)
- **Fix Effort Estimation**: Issues categorized as Low, Medium, or High effort
- **Enhanced Categories**: Added Maintainability, Testing, and Architecture categories
- **Executive Summaries**: Professional reports with code quality scores (0-10 scale)
- **Smart Tagging**: Automatic issue tagging with relevant keywords
- **Code Suggestions**: AI-generated code fixes with diff previews
- **Professional Output**: Rich markdown formatting with emojis and structured reports
- **Enhanced Security Analysis**: Improved detection of SQL injection, XSS, and other vulnerabilities

### Enhanced
- **Comment System**: Extended with confidence, tags, code suggestions, and effort estimation
- **Output Formatting**: Smart review provides executive summaries and actionable recommendations
- **Issue Prioritization**: Issues now grouped by severity with clear priority ordering
- **Context Analysis**: Improved symbol extraction and definition lookup

### Technical
- Added `SmartReviewPromptBuilder` for enhanced prompt engineering
- Enhanced `CommentSynthesizer` with summary generation capabilities
- Extended comment metadata with confidence, effort, and tagging systems
- Improved error handling and validation

## [0.3.0] - 2024-12-XX

### Added
- Configuration file support (.diffscope.yml)
- Enhanced Anthropic API support for all Claude models
- CLI options for temperature, max-tokens, and custom prompts
- Compare command for file-to-file analysis
- Symbol extraction and definition lookup
- Plugin system with builtin analyzers

### Fixed
- Anthropic API compatibility with latest Claude models
- Unused import and variable warnings
- Model detection logic for all providers

## [0.2.0] - 2024-XX-XX

### Added
- Multiple LLM provider support (OpenAI, Anthropic, Ollama)
- Git integration with branch comparison
- PR review capabilities with GitHub CLI
- Commit message suggestion
- Multiple output formats (JSON, Markdown, Patch)

## [0.1.0] - 2024-XX-XX

### Added
- Initial release with basic diff analysis
- OpenAI GPT integration
- Command-line interface
- Basic code review functionality