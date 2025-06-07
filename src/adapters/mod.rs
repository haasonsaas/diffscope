pub mod llm;
pub mod openai;
pub mod ollama;
pub mod anthropic;

pub use openai::OpenAIAdapter;
pub use ollama::OllamaAdapter;
pub use anthropic::AnthropicAdapter;