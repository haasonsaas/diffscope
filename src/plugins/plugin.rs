use crate::config::PluginConfig;
use crate::core::{Comment, UnifiedDiff};
use crate::plugins::{PostProcessor, PreAnalysis, PreAnalyzer};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub struct PluginManager {
    pre_analyzers: Vec<Arc<dyn PreAnalyzer>>,
    post_processors: Vec<Arc<dyn PostProcessor>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            pre_analyzers: Vec::new(),
            post_processors: Vec::new(),
        }
    }

    pub async fn load_builtin_plugins(&mut self, config: &PluginConfig) -> Result<()> {
        if config.eslint {
            self.register_pre_analyzer(Arc::new(crate::plugins::builtin::EslintAnalyzer::new()));
        }
        if config.semgrep {
            self.register_pre_analyzer(Arc::new(crate::plugins::builtin::SemgrepAnalyzer::new()));
        }
        if config.secret_scanner {
            self.register_pre_analyzer(Arc::new(crate::plugins::builtin::SecretScanner::new()));
        }
        if config.supply_chain {
            self.register_pre_analyzer(Arc::new(
                crate::plugins::builtin::SupplyChainAnalyzer::new(),
            ));
        }
        if config.rust_compile {
            self.register_pre_analyzer(Arc::new(
                crate::plugins::builtin::RustCompileAnalyzer::new(),
            ));
        }
        if !config.sarif_reports.is_empty() {
            self.register_pre_analyzer(Arc::new(crate::plugins::builtin::SarifAnalyzer::new(
                config.sarif_reports.clone(),
            )));
        }
        if config.duplicate_filter {
            self.register_post_processor(Arc::new(crate::plugins::builtin::DuplicateFilter::new()));
        }

        Ok(())
    }

    pub fn register_pre_analyzer(&mut self, analyzer: Arc<dyn PreAnalyzer>) {
        self.pre_analyzers.push(analyzer);
    }

    pub fn register_post_processor(&mut self, processor: Arc<dyn PostProcessor>) {
        self.post_processors.push(processor);
    }

    #[allow(dead_code)]
    pub async fn run_pre_analyzers(
        &self,
        diff: &UnifiedDiff,
        repo_path: &str,
    ) -> Result<PreAnalysis> {
        let mut analysis = PreAnalysis::default();

        for analyzer in &self.pre_analyzers {
            match analyzer.run(diff, repo_path).await {
                Ok(result) => analysis.extend(result),
                Err(e) => {
                    tracing::warn!("Pre-analyzer {} failed: {}", analyzer.id(), e);
                }
            }
        }

        Ok(analysis)
    }

    pub async fn run_pre_analyzers_for_review(
        &self,
        diffs: &[UnifiedDiff],
        repo_path: &str,
    ) -> Result<HashMap<PathBuf, PreAnalysis>> {
        let mut merged: HashMap<PathBuf, PreAnalysis> = HashMap::new();

        for analyzer in &self.pre_analyzers {
            match analyzer.run_batch(diffs, repo_path).await {
                Ok(results) => {
                    for (file_path, analysis) in results {
                        merged.entry(file_path).or_default().extend(analysis);
                    }
                }
                Err(e) => {
                    tracing::warn!("Pre-analyzer {} failed: {}", analyzer.id(), e);
                }
            }
        }

        Ok(merged)
    }

    pub async fn run_post_processors(
        &self,
        comments: Vec<Comment>,
        repo_path: &str,
    ) -> Result<Vec<Comment>> {
        let mut processed = comments;

        for processor in &self.post_processors {
            match processor.run(processed.clone(), repo_path).await {
                Ok(result) => processed = result,
                Err(e) => {
                    tracing::warn!("Post-processor {} failed: {}", processor.id(), e);
                }
            }
        }

        Ok(processed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_builtin_plugins_respects_config() {
        let mut manager = PluginManager::new();
        let config = PluginConfig {
            eslint: false,
            semgrep: true,
            duplicate_filter: false,
            secret_scanner: false,
            supply_chain: false,
            rust_compile: false,
            sarif_reports: Vec::new(),
        };

        manager.load_builtin_plugins(&config).await.unwrap();

        assert_eq!(manager.pre_analyzers.len(), 1);
        assert_eq!(manager.post_processors.len(), 0);
    }

    #[tokio::test]
    async fn load_builtin_plugins_registers_security_analyzers() {
        let mut manager = PluginManager::new();
        let config = PluginConfig {
            eslint: false,
            semgrep: false,
            duplicate_filter: false,
            secret_scanner: true,
            supply_chain: true,
            rust_compile: false,
            sarif_reports: Vec::new(),
        };

        manager.load_builtin_plugins(&config).await.unwrap();

        assert_eq!(manager.pre_analyzers.len(), 2);
        assert_eq!(manager.post_processors.len(), 0);
    }

    #[tokio::test]
    async fn load_builtin_plugins_registers_rust_compile_analyzer() {
        let mut manager = PluginManager::new();
        let config = PluginConfig {
            eslint: false,
            semgrep: false,
            duplicate_filter: false,
            secret_scanner: false,
            supply_chain: false,
            rust_compile: true,
            sarif_reports: Vec::new(),
        };

        manager.load_builtin_plugins(&config).await.unwrap();

        assert_eq!(manager.pre_analyzers.len(), 1);
        assert_eq!(manager.post_processors.len(), 0);
    }

    #[tokio::test]
    async fn load_builtin_plugins_registers_sarif_analyzer_when_reports_configured() {
        let mut manager = PluginManager::new();
        let config = PluginConfig {
            eslint: false,
            semgrep: false,
            duplicate_filter: false,
            secret_scanner: false,
            supply_chain: false,
            rust_compile: false,
            sarif_reports: vec!["codeql.sarif".to_string()],
        };

        manager.load_builtin_plugins(&config).await.unwrap();

        assert_eq!(manager.pre_analyzers.len(), 1);
        assert_eq!(manager.post_processors.len(), 0);
    }
}
