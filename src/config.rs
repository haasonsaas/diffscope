use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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