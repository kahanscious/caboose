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

#[cfg(test)]
mod tests {
    use super::*;

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
