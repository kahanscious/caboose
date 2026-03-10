//! Environment variable filtering — strip secrets before command execution.

use std::collections::HashSet;

/// Known secret environment variable patterns.
const SECRET_PATTERNS: &[&str] = &[
    "API_KEY",
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "CREDENTIAL",
    "PRIVATE_KEY",
];

/// Build a filtered environment map, stripping secret variables.
pub fn filtered_env(additional_secrets: &[String]) -> Vec<(String, String)> {
    let secret_names: HashSet<&str> = additional_secrets.iter().map(|s| s.as_str()).collect();

    std::env::vars()
        .filter(|(key, _)| {
            let upper = key.to_uppercase();
            // Keep if not a secret pattern
            !SECRET_PATTERNS.iter().any(|p| upper.contains(p))
                && !secret_names.contains(key.as_str())
        })
        .collect()
}

/// Check whether a given env var name would be filtered out.
#[allow(dead_code)]
pub fn is_secret(key: &str, additional_secrets: &[String]) -> bool {
    let upper = key.to_uppercase();
    SECRET_PATTERNS.iter().any(|p| upper.contains(p)) || additional_secrets.iter().any(|s| s == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_api_key_pattern() {
        assert!(is_secret("ANTHROPIC_API_KEY", &[]));
        assert!(is_secret("OPENAI_API_KEY", &[]));
        assert!(is_secret("my_api_key", &[]));
    }

    #[test]
    fn filters_secret_patterns() {
        assert!(is_secret("AWS_SECRET_ACCESS_KEY", &[]));
        assert!(is_secret("GITHUB_TOKEN", &[]));
        assert!(is_secret("DB_PASSWORD", &[]));
        assert!(is_secret("SSH_PRIVATE_KEY", &[]));
        assert!(is_secret("SOME_CREDENTIAL", &[]));
    }

    #[test]
    fn keeps_safe_variables() {
        assert!(!is_secret("HOME", &[]));
        assert!(!is_secret("PATH", &[]));
        assert!(!is_secret("TERM", &[]));
        assert!(!is_secret("SHELL", &[]));
        assert!(!is_secret("USER", &[]));
    }

    #[test]
    fn additional_secrets_exact_match() {
        let extra = vec!["MY_CUSTOM_VAR".to_string()];
        assert!(is_secret("MY_CUSTOM_VAR", &extra));
        assert!(!is_secret("MY_CUSTOM_VAR_2", &extra));
    }

    #[test]
    fn filtered_env_excludes_secrets() {
        // SAFETY: test runs single-threaded; no concurrent env access.
        unsafe {
            std::env::set_var("CABOOSE_TEST_API_KEY", "should_be_filtered");
            std::env::set_var("CABOOSE_TEST_SAFE", "should_remain");
        }

        let env = filtered_env(&[]);
        assert!(!env.iter().any(|(k, _)| k == "CABOOSE_TEST_API_KEY"));
        assert!(env.iter().any(|(k, _)| k == "CABOOSE_TEST_SAFE"));

        unsafe {
            std::env::remove_var("CABOOSE_TEST_API_KEY");
            std::env::remove_var("CABOOSE_TEST_SAFE");
        }
    }
}
