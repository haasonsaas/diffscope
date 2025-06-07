use anyhow::Result;
use regex::Regex;
use std::collections::HashSet;
use crate::adapters::llm::{LLMAdapter, LLMRequest};

#[allow(dead_code)]
pub struct InteractiveCommand {
    pub command: CommandType,
    pub args: Vec<String>,
    pub context: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum CommandType {
    Review,
    Ignore,
    Explain,
    Generate,
    Help,
    Config,
}

#[allow(dead_code)]
impl InteractiveCommand {
    pub fn parse(comment: &str) -> Option<Self> {
        let command_regex = Regex::new(r"@diffscope\s+(\w+)(?:\s+(.*))?").ok()?;
        
        if let Some(captures) = command_regex.captures(comment) {
            let command_str = captures.get(1)?.as_str();
            let args_str = captures.get(2).map(|m| m.as_str()).unwrap_or("");
            
            let command_type = match command_str.to_lowercase().as_str() {
                "review" => CommandType::Review,
                "ignore" => CommandType::Ignore,
                "explain" => CommandType::Explain,
                "generate" => CommandType::Generate,
                "help" => CommandType::Help,
                "config" => CommandType::Config,
                _ => return None,
            };
            
            let args = if args_str.is_empty() {
                Vec::new()
            } else {
                args_str.split_whitespace().map(String::from).collect()
            };
            
            Some(InteractiveCommand {
                command: command_type,
                args,
                context: None,
            })
        } else {
            None
        }
    }
    
    pub async fn execute(
        &self,
        adapter: &Box<dyn LLMAdapter>,
        diff_content: Option<&str>,
    ) -> Result<String> {
        match &self.command {
            CommandType::Review => self.execute_review(adapter, diff_content).await,
            CommandType::Ignore => self.execute_ignore(),
            CommandType::Explain => self.execute_explain(adapter, diff_content).await,
            CommandType::Generate => self.execute_generate(adapter).await,
            CommandType::Help => Ok(Self::get_help_text()),
            CommandType::Config => Ok(Self::get_config_info()),
        }
    }
    
    async fn execute_review(
        &self,
        adapter: &Box<dyn LLMAdapter>,
        diff_content: Option<&str>,
    ) -> Result<String> {
        if let Some(diff) = diff_content {
            let prompt = if self.args.is_empty() {
                format!("Review the following code changes:\n\n{}", diff)
            } else {
                let focus = self.args.join(" ");
                format!("Review the following code changes with focus on {}:\n\n{}", focus, diff)
            };
            
            let request = LLMRequest {
                system_prompt: "You are a code reviewer. Provide concise, actionable feedback.".to_string(),
                user_prompt: prompt,
                temperature: Some(0.3),
                max_tokens: Some(1000),
            };
            
            let response = adapter.complete(request).await?;
            Ok(format!("## 🔍 Code Review\n\n{}", response.content))
        } else {
            Ok("No diff content available for review.".to_string())
        }
    }
    
    fn execute_ignore(&self) -> Result<String> {
        if self.args.is_empty() {
            Ok("Please specify what to ignore (e.g., @diffscope ignore src/generated/)".to_string())
        } else {
            let patterns = self.args.join(", ");
            Ok(format!("✅ Will ignore: {}\n\nAdd these patterns to your .diffscope.yml for permanent configuration.", patterns))
        }
    }
    
    async fn execute_explain(
        &self,
        adapter: &Box<dyn LLMAdapter>,
        diff_content: Option<&str>,
    ) -> Result<String> {
        let context = if self.args.is_empty() {
            diff_content.unwrap_or("No specific context").to_string()
        } else {
            // Try to find specific line or section
            let target = self.args.join(" ");
            format!("Explain the following in the context of the code changes: {}", target)
        };
        
        let request = LLMRequest {
            system_prompt: "You are a helpful code explainer. Provide clear, educational explanations.".to_string(),
            user_prompt: format!("Explain this code or change:\n\n{}", context),
            temperature: Some(0.5),
            max_tokens: Some(800),
        };
        
        let response = adapter.complete(request).await?;
        Ok(format!("## 💡 Explanation\n\n{}", response.content))
    }
    
    async fn execute_generate(&self, adapter: &Box<dyn LLMAdapter>) -> Result<String> {
        if self.args.is_empty() {
            return Ok("Please specify what to generate (e.g., @diffscope generate tests)".to_string());
        }
        
        let target = self.args[0].as_str();
        let context = self.args[1..].join(" ");
        
        let (system_prompt, user_prompt) = match target {
            "tests" => (
                "You are a test generation expert. Generate comprehensive tests.",
                format!("Generate unit tests for the following context: {}", context)
            ),
            "docs" => (
                "You are a documentation expert. Generate clear, comprehensive documentation.",
                format!("Generate documentation for: {}", context)
            ),
            "types" => (
                "You are a TypeScript/type system expert. Generate proper type definitions.",
                format!("Generate type definitions for: {}", context)
            ),
            _ => (
                "You are a helpful code generator.",
                format!("Generate {} for: {}", target, context)
            ),
        };
        
        let request = LLMRequest {
            system_prompt: system_prompt.to_string(),
            user_prompt,
            temperature: Some(0.7),
            max_tokens: Some(1500),
        };
        
        let response = adapter.complete(request).await?;
        Ok(format!("## 🔨 Generated {}\n\n```\n{}\n```", target, response.content))
    }
    
    fn get_help_text() -> String {
        r#"## 🤖 DiffScope Interactive Commands

Available commands:

### Review
- `@diffscope review` - Review the current changes
- `@diffscope review security` - Focus review on security aspects
- `@diffscope review performance` - Focus on performance

### Ignore
- `@diffscope ignore src/generated/` - Ignore files matching pattern
- `@diffscope ignore *.test.js` - Ignore test files

### Explain
- `@diffscope explain` - Explain the overall changes
- `@diffscope explain line 42` - Explain specific line
- `@diffscope explain function_name` - Explain specific function

### Generate
- `@diffscope generate tests` - Generate unit tests
- `@diffscope generate docs` - Generate documentation
- `@diffscope generate types` - Generate type definitions

### Other
- `@diffscope help` - Show this help message
- `@diffscope config` - Show current configuration"#.to_string()
    }
    
    fn get_config_info() -> String {
        r#"## ⚙️ Current Configuration

To configure DiffScope behavior, create a `.diffscope.yml` file:

```yaml
model: gpt-4o
temperature: 0.2
max_tokens: 4000

# Ignore patterns
exclude_patterns:
  - "**/*.generated.*"
  - "**/node_modules/**"
  
# Path-specific rules  
paths:
  "src/api/**":
    focus: ["security", "validation"]
  "tests/**":
    focus: ["coverage", "assertions"]
```

Interactive commands respect these configurations."#.to_string()
    }
}

#[allow(dead_code)]
pub struct InteractiveProcessor {
    ignored_patterns: HashSet<String>,
}

#[allow(dead_code)]
impl InteractiveProcessor {
    pub fn new() -> Self {
        Self {
            ignored_patterns: HashSet::new(),
        }
    }
    
    pub fn add_ignore_pattern(&mut self, pattern: &str) {
        self.ignored_patterns.insert(pattern.to_string());
    }
    
    pub fn should_ignore(&self, path: &str) -> bool {
        self.ignored_patterns.iter().any(|pattern| {
            // Simple glob matching
            if pattern.contains('*') {
                let regex_pattern = pattern.replace("*", ".*");
                regex::Regex::new(&regex_pattern)
                    .map(|re| re.is_match(path))
                    .unwrap_or(false)
            } else {
                path.contains(pattern)
            }
        })
    }
}