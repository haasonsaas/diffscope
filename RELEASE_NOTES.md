# Release Notes - v0.5.0

📅 **Release Date**: 2025-06-06

## 📊 Summary

This release brings major new features inspired by CodeRabbit, including PR summary generation, interactive commands, changelog generation, and path-based configuration.

- 🎯 **Total Changes**: 4 major features
- ✨ **New Features**: 4
- 🐛 **Bug Fixes**: 0
- ⚠️ **Breaking Changes**: 0

## ✨ Highlights

### 1. PR Summary Generation
- Generate comprehensive executive summaries for pull requests
- Includes statistics, change analysis, and risk assessment
- Seamless GitHub integration with `diffscope pr --summary`

### 2. Interactive PR Commands
- Respond to PR comments with `@diffscope` commands
- Support for review, ignore, explain, generate, and help commands
- Makes code review more collaborative and interactive

### 3. Changelog & Release Notes Generation
- Automatically parse conventional commits
- Generate professional changelogs with `diffscope changelog`
- Support for both changelog and release notes formats
- Group changes by type with emoji support

### 4. Path-Based Configuration
- Configure review behavior per directory/file pattern
- Set custom focus areas, severity overrides, and prompts
- Support for exclude patterns and path-specific rules
- Example: Elevate all security issues to errors in API endpoints

## 🔧 Configuration

Create a `.diffscope.yml` file to customize behavior:

```yaml
# Path-specific rules
paths:
  "src/api/**":
    focus: [security, validation]
    severity_overrides:
      security: error
```

## 🚀 Getting Started

```bash
# Install the latest version
cargo install diffscope

# Generate a changelog
diffscope changelog --from v0.4.0

# Use path-based configuration
cp .diffscope.yml.example .diffscope.yml
```

## 👥 Contributors

- Jonathan Haas (@Haasonsaas)

## 📝 Full Changelog

For detailed changes, see the [full changelog](https://github.com/Haasonsaas/diffscope/compare/v0.4.4...v0.5.0).