//! Destination components, not complete paths. Sanitization is not uniqueness.
//! Callers must detect normalized/case-insensitive collisions before any writes.
use std::fmt;
use unicode_normalization::UnicodeNormalization;

/// Conservative UTF-8 byte budget per component, including any filename suffix.
pub const MAX_COMPONENT_BYTES: usize = 180;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedComponent {
    value: String,
    pub changed: bool,
}

impl SanitizedComponent {
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Portable collision hint, not a full model of any filesystem's collation.
    /// Does not handle every Unicode case-folding or platform alias.
    pub fn collision_key(&self) -> String {
        self.value.to_lowercase().nfc().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong { bytes: usize, limit: usize },
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "name is empty after sanitization"),
            Self::TooLong { bytes, limit } => write!(
                f,
                "name is {bytes} UTF-8 bytes; limit is {limit}; no truncation applied"
            ),
        }
    }
}
impl std::error::Error for NameError {}

/// Normalize to NFC, replace separators/forbidden/control characters with '_',
/// trim surrounding whitespace and trailing periods, and escape device names.
/// Reject empty and overlong names. This never infers missing metadata.
pub fn sanitize_component(raw: &str) -> Result<SanitizedComponent, NameError> {
    let normalized: String = raw
        .nfc()
        .map(|c| {
            if c.is_control()
                || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut value = normalized
        .trim()
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .to_owned();
    if value.is_empty() {
        return Err(NameError::Empty);
    }
    if reserved(&value) {
        value.insert(0, '_');
    }
    if value.len() > MAX_COMPONENT_BYTES {
        return Err(NameError::TooLong {
            bytes: value.len(),
            limit: MAX_COMPONENT_BYTES,
        });
    }
    Ok(SanitizedComponent {
        changed: value != raw,
        value,
    })
}

fn reserved(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Component, Path};

    #[test]
    fn preserves_readable_names_and_composes_unicode_without_transliteration() {
        let unchanged = sanitize_component("Björk - Jóga").unwrap();
        assert_eq!(unchanged.as_str(), "Björk - Jóga");
        assert!(!unchanged.changed);
        let composed = sanitize_component("Cafe\u{301}").unwrap();
        assert_eq!(composed.as_str(), "Café");
        assert!(composed.changed);
    }

    #[test]
    fn sanitizes_separators_controls_and_trailing_characters() {
        let name = sanitize_component("  A/B\\C:D*E?F\"G<H>I|J\n.  ").unwrap();
        assert_eq!(name.as_str(), "A_B_C_D_E_F_G_H_I_J_");
        assert!(name.changed);
        let components: Vec<_> = Path::new(name.as_str()).components().collect();
        assert!(matches!(components.as_slice(), [Component::Normal(_)]));
        assert_eq!(sanitize_component("a\u{202e}b").unwrap().as_str(), "a_b");
    }

    #[test]
    fn rejects_empty_dot_and_parent_components() {
        for raw in ["", " ", ".", "..", "...", " . . "] {
            assert_eq!(sanitize_component(raw), Err(NameError::Empty), "{raw:?}");
        }
        for raw in ["/", "../song", "C:\\music", "\0"] {
            let name = sanitize_component(raw).unwrap();
            assert!(matches!(
                Path::new(name.as_str())
                    .components()
                    .collect::<Vec<_>>()
                    .as_slice(),
                [Component::Normal(_)]
            ));
        }
    }

    #[test]
    fn escapes_reserved_device_names_including_extensions() {
        for raw in [
            "CON",
            "con.flac",
            "AUX",
            "nul.txt",
            "PRN",
            "COM1",
            "Lpt9.flac",
            "COM¹",
            "LPT².wav",
            "CONIN$",
            "CONOUT$",
        ] {
            assert!(
                sanitize_component(raw).unwrap().as_str().starts_with('_'),
                "{raw}"
            );
        }
        for raw in ["Console", "COM10", "LPT0", "AUXiliary"] {
            assert_eq!(sanitize_component(raw).unwrap().as_str(), raw);
        }
    }

    #[test]
    fn enforces_byte_limit_without_splitting_or_truncating_unicode() {
        assert_eq!(
            sanitize_component(&"a".repeat(180)).unwrap().as_str().len(),
            180
        );
        assert!(matches!(
            sanitize_component(&"a".repeat(181)),
            Err(NameError::TooLong { bytes: 181, .. })
        ));
        assert!(sanitize_component(&"é".repeat(90)).is_ok());
        assert!(matches!(
            sanitize_component(&"é".repeat(91)),
            Err(NameError::TooLong { bytes: 182, .. })
        ));
    }

    #[test]
    fn exposes_collisions_and_is_idempotent() {
        for (a, b) in [
            ("a/b", "a\\b"),
            ("CON", "_CON"),
            ("Café", "Cafe\u{301}"),
            ("Artist", "artist"),
            ("Song. ", "Song"),
        ] {
            let first = sanitize_component(a).unwrap();
            let second = sanitize_component(b).unwrap();
            assert_eq!(first.collision_key(), second.collision_key(), "{a} / {b}");
            let again = sanitize_component(first.as_str()).unwrap();
            assert_eq!(again.as_str(), first.as_str());
            assert!(!again.changed);
        }
    }
}
