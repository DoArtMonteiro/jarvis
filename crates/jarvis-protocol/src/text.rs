//! Bounded, single-line text supplied by people or workers.
//!
//! Text that crosses a trust boundary and may later be shown in a terminal
//! must not carry control characters: they could forge log lines or move
//! the cursor. These types reject them at decode time instead of escaping
//! them on every display path.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! bounded_text {
    ($(#[$doc:meta])* $name:ident, $kind:literal, min = $min:expr, max = $max:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Maximum length in characters.
            pub const MAX_CHARS: usize = $max;

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                let chars = value.chars().count();
                if !($min..=$max).contains(&chars) {
                    return Err(format!(
                        "{} must be {} to {} characters",
                        $kind, $min, $max
                    ));
                }
                if value.chars().any(char::is_control) {
                    return Err(format!("{} must not contain control characters", $kind));
                }
                Ok(Self(value))
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::try_from(value.to_owned())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

bounded_text!(
    /// What a person asks JARVIS to do. Submitted over the local RPC
    /// interface and passed to the worker unchanged.
    Goal,
    "goal",
    min = 1,
    max = 2000
);

bounded_text!(
    /// A short single-line report, such as a worker's job summary or the
    /// reason a person gives for declining an approval.
    Summary,
    "summary",
    min = 0,
    max = 1000
);

impl Summary {
    /// A valid summary built from any text: control characters are escaped
    /// and the result is cut to [`Summary::MAX_CHARS`] characters.
    pub fn lossy(text: &str) -> Self {
        let mut out = String::new();
        let mut count = 0;
        for c in text.chars() {
            let piece: String = if c.is_control() {
                c.escape_default().collect()
            } else {
                c.to_string()
            };
            let len = piece.chars().count();
            if count + len > Self::MAX_CHARS {
                break;
            }
            count += len;
            out.push_str(&piece);
        }
        Self(out)
    }
}

/// Characters that change how text displays without being seen: bidi
/// overrides and isolates, zero-width characters, the soft hyphen and the
/// byte-order mark. With them, the text a person approves could read
/// differently from the text that is used.
pub(crate) fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// Whether clipboard text may contain `c`: an allowlist, so that what a
/// person approves reads the same as what is pasted. Newline, tab, space,
/// printable ASCII, letters and digits of any script (by their Unicode
/// properties), and common typographic punctuation. Everything else is
/// refused, including format, separator, private-use and unassigned
/// characters, variation selectors, tag characters, combining marks that
/// are not letters, spaces other than U+0020, and the letters that render as
/// nothing (the Hangul fillers). Lookalike letters from other scripts remain
/// possible; they are visible, and the SHA-256 tells two texts apart.
pub(crate) fn is_plain(c: char) -> bool {
    match c {
        '\n' | '\t' | ' ' => true,
        c if c.is_ascii_graphic() => true,
        '\u{115F}' | '\u{1160}' | '\u{3164}' | '\u{FFA0}' => false,
        c if c.is_alphanumeric() => true,
        '\u{00A1}'..='\u{00AC}'
        | '\u{00AE}'..='\u{00BF}'
        | '\u{00D7}'
        | '\u{00F7}'
        | '\u{2010}'..='\u{2015}'
        | '\u{2018}'..='\u{2022}'
        | '\u{2026}'
        | '\u{2030}'
        | '\u{2039}'
        | '\u{203A}'
        | '\u{20AC}'
        | '\u{2122}' => true,
        _ => false,
    }
}

/// Check clipboard text: `min..=max` characters, each one allowed by
/// [`is_plain`].
pub(crate) fn check_plain(kind: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    let chars = value.chars().count();
    if !(min..=max).contains(&chars) {
        return Err(format!("{kind} must be {min} to {max} characters"));
    }
    match value.chars().find(|&c| !is_plain(c)) {
        Some(c) => Err(format!(
            "{kind} must not contain {}: only letters, digits, common punctuation, \
             spaces, tabs and newlines are allowed",
            c.escape_unicode()
        )),
        None => Ok(()),
    }
}

/// Check text a person will approve: `min..=max` characters, no invisible
/// formatting characters, and no control characters except newline and tab
/// when `allow_layout` is set.
pub(crate) fn check_visible(
    kind: &str,
    value: &str,
    min: usize,
    max: usize,
    allow_layout: bool,
) -> Result<(), String> {
    let chars = value.chars().count();
    if !(min..=max).contains(&chars) {
        return Err(format!("{kind} must be {min} to {max} characters"));
    }
    for c in value.chars() {
        if c.is_control() && !(allow_layout && (c == '\n' || c == '\t')) {
            return Err(format!("{kind} must not contain control characters"));
        }
        if is_invisible(c) {
            return Err(format!(
                "{kind} must not contain invisible formatting characters"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_control_characters_and_bounds_length() {
        assert!(Goal::try_from("describe the runtime".to_owned()).is_ok());
        assert!(Goal::try_from(String::new()).is_err());
        assert!(Goal::try_from("x".repeat(2001)).is_err());
        assert!(
            Goal::try_from("é".repeat(2000)).is_ok(),
            "limit counts characters"
        );
        for bad in ["a\nb", "a\u{1b}[31m", "a\rb", "a\u{7}"] {
            assert!(Goal::try_from(bad.to_owned()).is_err(), "{bad:?}");
            assert!(Summary::try_from(bad.to_owned()).is_err(), "{bad:?}");
        }
        assert!(Summary::try_from(String::new()).is_ok());
    }

    #[test]
    fn lossy_summary_is_always_valid() {
        let summary = Summary::lossy("a\nb\u{1b}c");
        assert_eq!(summary.as_str(), "a\\nb\\u{1b}c");
        let long = Summary::lossy(&"x".repeat(5000));
        assert_eq!(long.as_str().chars().count(), Summary::MAX_CHARS);
        assert!(Summary::try_from(long.as_str().to_owned()).is_ok());
    }
}
