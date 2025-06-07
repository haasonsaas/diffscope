pub mod diff_parser;
pub mod context;
pub mod prompt;
pub mod comment;
pub mod git;
pub mod commit_prompt;
pub mod smart_review_prompt;

pub use diff_parser::{DiffParser, UnifiedDiff};
pub use context::{ContextFetcher, LLMContextChunk, ContextType};
pub use prompt::PromptBuilder;
pub use comment::{Comment, CommentSynthesizer};
pub use git::GitIntegration;
pub use commit_prompt::CommitPromptBuilder;
pub use smart_review_prompt::SmartReviewPromptBuilder;