// Conflict testing playground for subagent merge behavior.

/// Simple user struct for testing.
#[derive(Debug, Clone)]
pub struct User {
    pub id: u64,
    pub name: String,
}

/// Simple context struct for testing.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub version: String,
    pub enabled: bool,
}

/// A large function that transforms a user into a string.
// This function is intentionally long (>=30 lines) to allow different subagents
/// to edit different regions without touching the same exact lines.
pub fn shared_transform(user: &User) -> String {
    // Line 1: Start with a greeting
    let mut result = String::new();
    let start = std::time::Instant::now();
    result.push_str("User Transform v2.0 - Modified by Agent A\n");
    result.push_str(&format!("Timestamp: {:?}\n", std::time::SystemTime::now()));
    
    // Line 2-5: User basic info
    result.push_str(&format!("ID: {}\n", user.id));
    result.push_str(&format!("Name: {}\n", user.name));
    result.push_str("------------------------\n");
    
    // Line 6-10: Process name characters
    let name_upper = user.name.to_uppercase();
    let name_lower = user.name.to_lowercase();
    let name_len = user.name.len();
    result.push_str(&format!("Upper: {}\n", name_upper));
    result.push_str(&format!("Lower: {}\n", name_lower));
    result.push_str(&format!("Length: {}\n", name_len));
    result.push_str("------------------------\n");
    
    // Line 11-15: ID transformations
    let id_squared = user.id * user.id;
    let id_cubed = id_squared * user.id;
    let id_double = user.id * 2;
    result.push_str(&format!("ID^2: {}\n", id_squared));
    result.push_str(&format!("ID^3: {}\n", id_cubed));
    result.push_str(&format!("ID*2: {}\n", id_double));
    result.push_str("------------------------\n");
    
    // Line 16-20: String manipulation
    let reversed: String = user.name.chars().rev().collect();
    let trimmed = user.name.trim();
    let is_empty = user.name.is_empty();
    result.push_str(&format!("Reversed: {}\n", reversed));
    result.push_str(&format!("Trimmed: '{}'\n", trimmed));
    result.push_str(&format!("Is empty: {}\n", is_empty));
    result.push_str("------------------------\n");
    
    // Line 21-25: More complex logic
    let chars: Vec<char> = user.name.chars().collect();
    let vowel_count = chars.iter().filter(|c| matches!(c, 'a'|'e'|'i'|'o'|'u'|'A'|'E'|'I'|'O'|'U')).count();
    let consonant_count = chars.len() - vowel_count;
    result.push_str(&format!("Vowels: {}\n", vowel_count));
    result.push_str(&format!("Consonants: {}\n", consonant_count));
    result.push_str("------------------------\n");
    
    // Line 26-30: Final assembly and return
    result.push_str(&format!("Transformed at: {:?}\n", std::time::SystemTime::now()));
    let duration = start.elapsed();
    result.push_str(&format!("Processing time: {:?} (modified by Agent B)\n", duration));
    result.push_str("=== Transform Complete v2.0 - Modified by Agent B (end changed) ===\n");
    result
}

/// A helper function that does NOT call shared_transform.
/// This function provides a simple string representation of a User without using the large transform.
pub fn simple_user_repr(user: &User) -> String {
    format!("User {{ id: {}, name: {} }}", user.id, user.name)
}