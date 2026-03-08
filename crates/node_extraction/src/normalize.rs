//! Normalization for entity values (deduplication).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Normalize a value for deduplication.
/// - lowercase emails
/// - collapse whitespace
/// - trim punctuation
/// - normalize company suffix punctuation
/// - normalize phone to digits only (for comparison)
pub fn normalize_value(entity_type: &str, value: &str) -> String {
    let s = value.trim();
    let s = collapse_whitespace(s);
    match entity_type {
        "email" => s.to_lowercase(),
        "phone" => digits_only(&s),
        "company" => {
            let s = s.to_lowercase();
            normalize_company_suffix(&s)
        }
        "person" => s.to_lowercase(),
        "money" => {
            // Keep digits, decimal, for dedup - strip currency symbols
            s.chars()
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                .collect::<String>()
        }
        "date" => s.to_lowercase(),
        "invoice_number" | "quote_number" => {
            let s = s.to_uppercase();
            collapse_whitespace(&s)
        }
        _ => s.to_lowercase(),
    }
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn digits_only(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

fn normalize_company_suffix(s: &str) -> String {
    let s = s
        .trim_end_matches('.')
        .replace(" limited", " ltd")
        .replace(" limited.", " ltd")
        .replace(" corporation", " corp")
        .replace(" incorporated", " inc");
    collapse_whitespace(&s)
}

/// Normalize a phrase for vocabulary lookup (lowercase, collapse whitespace).
pub fn normalize_phrase_for_vocab(s: &str) -> String {
    collapse_whitespace(s.trim()).to_lowercase()
}

/// Stable hash of normalized value for entity_id when value is very long.
#[allow(dead_code)]
pub fn hash_for_id(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_email() {
        assert_eq!(
            normalize_value("email", "John@Example.COM"),
            "john@example.com"
        );
    }

    #[test]
    fn normalize_phone() {
        assert_eq!(normalize_value("phone", "+44 20 7123 4567"), "442071234567");
        assert_eq!(normalize_value("phone", "(020) 7123-4567"), "02071234567");
    }

    #[test]
    fn normalize_company() {
        assert!(normalize_value("company", "Acme Limited").contains("ltd"));
        assert!(normalize_value("company", "Acme  Corp.  ").contains("corp"));
    }

    #[test]
    fn collapse_whitespace_internal() {
        assert_eq!(collapse_whitespace("a   b   c"), "a b c");
    }
}
