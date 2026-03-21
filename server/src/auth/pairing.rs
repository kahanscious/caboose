use std::time::{Duration, Instant};

use rand::Rng;

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 6;
const CODE_TTL: Duration = Duration::from_secs(5 * 60);

/// A single one-time pairing code.
pub struct PairingCode {
    pub code: String,
    pub created_at: Instant,
    pub used: bool,
}

impl PairingCode {
    fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= CODE_TTL
    }

    fn is_valid(&self) -> bool {
        !self.used && !self.is_expired()
    }
}

/// In-memory manager for one-time pairing codes.
#[derive(Default)]
pub struct PairingManager {
    codes: Vec<PairingCode>,
}

impl PairingManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remove expired or used codes, generate and store a new code, return it.
    pub fn generate(&mut self) -> String {
        self.codes.retain(|c| c.is_valid());
        let code = generate_code();
        self.codes.push(PairingCode {
            code: code.clone(),
            created_at: Instant::now(),
            used: false,
        });
        code
    }

    /// Validate `code`.  If found and valid, marks it as used and returns
    /// `true`.  Returns `false` for unknown, expired, or already-used codes.
    pub fn validate(&mut self, code: &str) -> bool {
        let upper = code.to_uppercase();
        if let Some(entry) = self
            .codes
            .iter_mut()
            .find(|c| c.code == upper && c.is_valid())
        {
            entry.used = true;
            return true;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn generate_code() -> String {
    let mut rng = rand::thread_rng();
    (0..CODE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..CODE_ALPHABET.len());
            CODE_ALPHABET[idx] as char
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generate_produces_6_char_code() {
        let mut mgr = PairingManager::new();
        let code = mgr.generate();
        assert_eq!(code.len(), CODE_LEN);
        // Every character must be from the allowed alphabet.
        for ch in code.chars() {
            assert!(
                CODE_ALPHABET.contains(&(ch as u8)),
                "unexpected char '{ch}' in code"
            );
        }
    }

    #[test]
    fn validate_burns_code() {
        let mut mgr = PairingManager::new();
        let code = mgr.generate();
        assert!(mgr.validate(&code), "first validation should succeed");
        assert!(!mgr.validate(&code), "second validation should fail");
    }

    #[test]
    fn validate_rejects_wrong_code() {
        let mut mgr = PairingManager::new();
        let _code = mgr.generate();
        assert!(!mgr.validate("ZZZZZZ"));
    }

    #[test]
    fn codes_are_unique() {
        let mut mgr = PairingManager::new();
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let code = mgr.generate();
            assert!(seen.insert(code), "duplicate code generated");
        }
    }
}
