mod eslint;
mod semgrep;
mod duplicate_filter;

pub use eslint::EslintAnalyzer;
pub use semgrep::SemgrepAnalyzer;
pub use duplicate_filter::DuplicateFilter;