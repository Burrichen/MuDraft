//! Text comparison shared by imports, tags, and search.

use unicode_normalization::UnicodeNormalization;

/// Canonical comparison form: Unicode NFC, case-folded, whitespace collapsed.
/// "Björk" (composed or decomposed), " björk " and "BJÖRK" all compare equal.
pub fn norm(s: &str) -> String {
    s.nfc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Trimmed, NFC, internal whitespace collapsed; case preserved (for display names).
pub fn tidy(s: &str) -> String {
    s.nfc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_case_space_and_unicode_form() {
        assert_eq!(
            norm("  Bjo\u{308}rk  Guðmundsdóttir "),
            norm("BJÖRK guðmundsdóttir")
        );
        assert_eq!(tidy("  Road   trip "), "Road trip");
    }
}
