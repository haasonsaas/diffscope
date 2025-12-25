use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_model")]
    pub model: String,
    
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    
    pub system_prompt: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    
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
            system_prompt: None,
            api_key: None,
            base_url: None,
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

        config.normalize();

        assert_eq!(config.model, default_model());
        assert_eq!(config.temperature, default_temperature());
        assert_eq!(config.max_tokens, default_max_tokens());
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

fn default_true() -> bool {
    true
}
