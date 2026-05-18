use std::collections::HashMap;
use std::path::PathBuf;

#[path = "pattern_repositories/checkout.rs"]
mod checkout;
#[path = "pattern_repositories/git.rs"]
mod git;
#[path = "pattern_repositories/local.rs"]
mod local;
#[path = "pattern_repositories/run.rs"]
mod run;

pub type PatternRepositoryMap = HashMap<String, PathBuf>;

pub use run::resolve_pattern_repositories;

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::config;

    use super::git::{is_git_source, is_safe_git_url};
    use super::run::resolve_pattern_repositories_with;

    #[test]
    fn test_is_git_source_https() {
        assert!(is_git_source("https://github.com/org/repo.git"));
        assert!(is_git_source("https://github.com/org/repo"));
    }

    #[test]
    fn test_is_git_source_ssh() {
        assert!(is_git_source("git@github.com:org/repo.git"));
    }

    #[test]
    fn test_is_git_source_http_with_git_suffix() {
        assert!(is_git_source("http://example.com/repo.git"));
    }

    #[test]
    fn test_is_git_source_rejects_local_paths() {
        assert!(!is_git_source("/tmp/evil"));
        assert!(!is_git_source("../relative/path"));
        assert!(!is_git_source("file:///etc/passwd"));
    }

    #[test]
    fn test_is_git_source_rejects_other_schemes() {
        assert!(!is_git_source("ftp://example.com/repo.git"));
    }

    #[test]
    fn test_is_git_source_accepts_ssh() {
        assert!(is_git_source("ssh://example.com/repo"));
    }

    #[test]
    fn test_is_safe_git_url_allows_https() {
        assert!(is_safe_git_url("https://github.com/org/repo"));
        assert!(is_safe_git_url("https://gitlab.com/org/repo.git"));
    }

    #[test]
    fn test_is_safe_git_url_allows_ssh() {
        assert!(is_safe_git_url("git@github.com:org/repo.git"));
        assert!(is_safe_git_url("ssh://example.com/repo"));
        assert!(is_safe_git_url("ssh://git@gitlab.internal/org/rules.git"));
    }

    #[test]
    fn test_is_safe_git_url_rejects_file_urls() {
        assert!(!is_safe_git_url("file:///etc/passwd"));
        assert!(!is_safe_git_url("/tmp/evil"));
        assert!(!is_safe_git_url("../traversal"));
    }

    #[test]
    fn test_is_safe_git_url_rejects_arbitrary_schemes() {
        assert!(!is_safe_git_url("ftp://example.com/repo"));
        assert!(!is_safe_git_url("gopher://example.com/repo"));
    }

    #[test]
    fn test_is_safe_git_url_rejects_http_without_git_suffix() {
        assert!(!is_safe_git_url("http://example.com/repo"));
    }

    #[test]
    fn test_resolve_pattern_repositories_accepts_repo_relative_local_paths() {
        let tempdir = tempfile::tempdir().unwrap();
        let repo_root = tempdir.path();
        let local_repo = repo_root.join("patterns/local-rules");
        std::fs::create_dir_all(&local_repo).unwrap();

        let mut config = config::Config::default();
        config.pattern_repositories = vec![config::PatternRepositoryConfig {
            source: "patterns/local-rules".to_string(),
            ..Default::default()
        }];

        let resolved = resolve_pattern_repositories_with(&config, repo_root, |_| None);

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved.get("patterns/local-rules"),
            Some(&local_repo.canonicalize().unwrap())
        );
    }

    #[test]
    fn test_resolve_pattern_repositories_rejects_parent_traversal_local_paths() {
        let tempdir = tempfile::tempdir().unwrap();
        let repo_root = tempdir.path().join("repo");
        let outside_repo = tempdir.path().join("outside-rules");
        std::fs::create_dir_all(&repo_root).unwrap();
        std::fs::create_dir_all(&outside_repo).unwrap();

        let mut config = config::Config::default();
        config.pattern_repositories = vec![config::PatternRepositoryConfig {
            source: "../outside-rules".to_string(),
            ..Default::default()
        }];

        let resolved = resolve_pattern_repositories_with(&config, &repo_root, |_| None);

        assert!(resolved.is_empty());
    }

    #[test]
    fn test_resolve_pattern_repositories_rejects_absolute_local_paths_outside_repo() {
        let tempdir = tempfile::tempdir().unwrap();
        let repo_root = tempdir.path().join("repo");
        let outside_repo = tempdir.path().join("outside-rules");
        std::fs::create_dir_all(&repo_root).unwrap();
        std::fs::create_dir_all(&outside_repo).unwrap();

        let mut config = config::Config::default();
        config.pattern_repositories = vec![config::PatternRepositoryConfig {
            source: outside_repo.display().to_string(),
            ..Default::default()
        }];

        let resolved = resolve_pattern_repositories_with(&config, &repo_root, |_| None);

        assert!(resolved.is_empty());
    }

    #[test]
    fn test_resolve_pattern_repositories_accepts_git_sources_via_checkout_helper() {
        let tempdir = tempfile::tempdir().unwrap();
        let repo_root = tempdir.path();
        let checkout_path = repo_root.join("cloned-rules");
        std::fs::create_dir_all(&checkout_path).unwrap();

        let mut config = config::Config::default();
        let source = "https://github.com/example/rules.git".to_string();
        config.pattern_repositories = vec![config::PatternRepositoryConfig {
            source: source.clone(),
            ..Default::default()
        }];

        let checkout_calls = RefCell::new(Vec::new());
        let resolved = resolve_pattern_repositories_with(&config, repo_root, |candidate| {
            checkout_calls.borrow_mut().push(candidate.to_string());
            Some(checkout_path.clone())
        });

        assert_eq!(checkout_calls.into_inner(), vec![source.clone()]);
        assert_eq!(resolved.get(&source), Some(&checkout_path));
    }

    #[test]
    fn test_resolve_pattern_repositories_skips_broken_sources() {
        let tempdir = tempfile::tempdir().unwrap();
        let repo_root = tempdir.path();

        let mut config = config::Config::default();
        config.pattern_repositories = vec![
            config::PatternRepositoryConfig {
                source: "missing/local-rules".to_string(),
                ..Default::default()
            },
            config::PatternRepositoryConfig {
                source: "https://github.com/example/broken.git".to_string(),
                ..Default::default()
            },
        ];

        let checkout_calls = RefCell::new(Vec::new());
        let resolved = resolve_pattern_repositories_with(&config, repo_root, |candidate| {
            checkout_calls.borrow_mut().push(candidate.to_string());
            None
        });

        assert!(resolved.is_empty());
        assert_eq!(
            checkout_calls.into_inner(),
            vec!["https://github.com/example/broken.git".to_string()]
        );
    }
}
