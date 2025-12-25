use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_model")]
    pub model: String,

    #[serde(default = "default_temperature")]
    pub temperature: f32,

    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,

    #[serde(default = "default_max_context_chars")]
    pub max_context_chars: usize,

    #[serde(default = "default_max_diff_chars")]
    pub max_diff_chars: usize,

    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,

    #[serde(default)]
    pub review_profile: Option<String>,

    #[serde(default)]
    pub review_instructions: Option<String>,

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

    #[serde(default)]
    pub symbol_index_lsp_command: Option<String>,

    #[serde(default = "default_symbol_index_lsp_languages")]
    pub symbol_index_lsp_languages: HashMap<String, String>,

    #[serde(default = "default_feedback_path")]
    pub feedback_path: PathBuf,

    pub system_prompt: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,

    #[serde(default)]
    pub openai_use_responses: Option<bool>,

    #[serde(default)]
    pub plugins: PluginConfig,

    #[serde(default)]
    pub exclude_patterns: Vec<String>,

    #[serde(default)]
    pub paths: HashMap<String, PathConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct PluginConfig {
    #[serde(default = "default_true")]
    pub eslint: bool,

    #[serde(default = "default_true")]
    pub semgrep: bool,

    #[serde(default = "default_true")]
    pub duplicate_filter: bool,
}

impl Default for PathConfig {
    fn default() -> Self {
        Self {
            focus: Vec::new(),
            ignore_patterns: Vec::new(),
            extra_context: Vec::new(),
            system_prompt: None,
            review_instructions: None,
            severity_overrides: HashMap::new(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: default_model(),
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            max_context_chars: default_max_context_chars(),
            max_diff_chars: default_max_diff_chars(),
            min_confidence: default_min_confidence(),
            review_profile: None,
            review_instructions: None,
            smart_review_summary: true,
            smart_review_diagram: false,
            symbol_index: true,
            symbol_index_provider: default_symbol_index_provider(),
            symbol_index_max_files: default_symbol_index_max_files(),
            symbol_index_max_bytes: default_symbol_index_max_bytes(),
            symbol_index_max_locations: default_symbol_index_max_locations(),
            symbol_index_lsp_command: None,
            symbol_index_lsp_languages: default_symbol_index_lsp_languages(),
            feedback_path: default_feedback_path(),
            system_prompt: None,
            api_key: None,
            base_url: None,
            openai_use_responses: None,
            plugins: PluginConfig::default(),
            exclude_patterns: Vec::new(),
            paths: HashMap::new(),
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

    pub fn merge_with_cli(&mut self, cli_model: Option<String>, cli_prompt: Option<String>) {
        if let Some(model) = cli_model {
            self.model = model;
        }
        if let Some(prompt) = cli_prompt {
            self.system_prompt = Some(prompt);
        }
    }

    pub fn normalize(&mut self) {
        if self.model.trim().is_empty() {
            self.model = default_model();
        }

        if !self.temperature.is_finite() || self.temperature < 0.0 || self.temperature > 2.0 {
            self.temperature = default_temperature();
        }

        if self.max_tokens == 0 {
            self.max_tokens = default_max_tokens();
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

        let provider = self.symbol_index_provider.trim().to_lowercase();
        if provider.is_empty() || !matches!(provider.as_str(), "regex" | "lsp") {
            self.symbol_index_provider = default_symbol_index_provider();
        } else {
            self.symbol_index_provider = provider;
        }

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
    }

    pub fn get_path_config(&self, file_path: &PathBuf) -> Option<&PathConfig> {
        let file_path_str = file_path.to_string_lossy();

        // Find the most specific matching path
        let mut best_match: Option<(&String, &PathConfig)> = None;

        for (pattern, config) in &self.paths {
            if self.path_matches(&file_path_str, pattern) {
                // Keep the most specific match (longest pattern)
                if best_match.is_none() || pattern.len() > best_match.unwrap().0.len() {
                    best_match = Some((pattern, config));
                }
            }
        }

        best_match.map(|(_, config)| config)
    }

    pub fn should_exclude(&self, file_path: &PathBuf) -> bool {
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

    fn path_matches(&self, path: &str, pattern: &str) -> bool {
        // Simple glob matching
        if pattern.contains('*') {
            if let Ok(glob_pattern) = glob::Pattern::new(pattern) {
                glob_pattern.matches(path)
            } else {
                false
            }
        } else {
            // Direct path prefix matching
            path.starts_with(pattern)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_clamps_values() {
        let mut config = Config::default();
        config.model = "   ".to_string();
        config.temperature = 5.0;
        config.max_tokens = 0;
        config.min_confidence = 2.0;
        config.review_profile = Some("ASSERTIVE".to_string());

        config.normalize();

        assert_eq!(config.model, default_model());
        assert_eq!(config.temperature, default_temperature());
        assert_eq!(config.max_tokens, default_max_tokens());
        assert_eq!(config.min_confidence, 1.0);
        assert_eq!(config.review_profile.as_deref(), Some("assertive"));
    }
}

fn default_model() -> String {
    "gpt-4o".to_string()
}

fn default_temperature() -> f32 {
    0.2
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

fn default_min_confidence() -> f32 {
    0.0
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

fn default_true() -> bool {
    true
}
