# Changelog

All notable changes to this project will be documented in this file.

## [0.5.1] - 2025-06-06

### Added
- **Pre-built Binaries**: Automated binary distribution for all major platforms
- **Release Automation**: GitHub Actions workflow for multi-platform builds
- **Easy Installation**: One-line install scripts for Linux/macOS and Windows
- **Docker Multi-arch**: Support for AMD64 and ARM64 architectures
- **Cross-compilation**: Build configuration for 6 different target platforms

### Changed
- **Binary Size**: Optimized release builds with LTO and stripping for smaller downloads
- **Docker Image**: Switched to Alpine Linux for minimal container size
- **Installation Docs**: Added comprehensive binary installation instructions

### Supported Platforms
- Linux x86_64 (GNU and MUSL variants)
- Linux ARM64
- macOS Intel (x86_64)
- macOS Apple Silicon (ARM64)
- Windows x86_64 (MSVC)

## [0.5.0] - 2025-06-06

### Added
- **PR Summary Generation**: Create executive summaries with statistics and risk analysis
- **Interactive Commands**: Respond to PR comments with @diffscope commands
- **Changelog Generation**: Generate changelogs and release notes from git history
- **Path-Based Configuration**: Customize review behavior for different directories
- **Focus Areas**: Configure specific review focuses per path (security, performance, etc.)
- **Severity Overrides**: Elevate or downgrade issue severity based on file paths

### Changed
- **Smart Review**: Enhanced with confidence scoring and fix effort estimation
- **Output Format**: Improved markdown with emojis and professional formatting

## [0.4.4] - 2025-06-06

### Fixed
- Updated changelog to remove specific references

## [0.4.3] - 2025-06-06

### Changed
- Renamed example security fixes file to generic template
- Cleaned up example files for broader usage

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