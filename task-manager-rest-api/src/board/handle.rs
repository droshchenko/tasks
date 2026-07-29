/// How many digits a task number is padded to when it is shown to a person.
///
/// Padding is cosmetic — `RMS-000042` sorts and reads better in a list than `RMS-42` — and it is
/// deliberately not part of the identity: [`parse_task_handle`] accepts either spelling, because a
/// human quoting an id from a chat writes `RMS-42`.
const NUMBER_WIDTH: usize = 6;

/// The characters a prefix may be made of.
///
/// `-` is excluded on purpose: it is the separator in a handle, and allowing it would make
/// `AB-C-7` ambiguous between prefix `AB` / number `C-7` and prefix `AB-C` / number `7`. Refusing
/// it at the point a prefix is set costs one validation and buys an unambiguous parse forever.
pub fn is_valid_prefix(prefix: &str) -> bool {
    !prefix.is_empty()
        && prefix.len() <= 16
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Build the handle a person sees: `RMS` + 42 -> `RMS-000042`.
pub fn compose_task_handle(prefix: &str, number: i64) -> String {
    format!("{}-{:0width$}", prefix, number, width = NUMBER_WIDTH)
}

/// One half of a parsed handle. Named rather than a tuple so a caller cannot swap the two.
#[derive(Debug, PartialEq, Eq)]
pub struct ParsedTaskHandle {
    pub prefix: String,
    pub number: i64,
}

/// Split `RMS-42` / `RMS-000042` / `rms-42` into its prefix and number.
///
/// Returns `None` for anything that is not a handle at all — no separator, an empty half, a
/// non-numeric or non-positive number, or a prefix that no project could legally hold. The prefix
/// comes back upper-cased, which is the spelling prefixes are stored in, so a caller never has to
/// normalise before looking one up.
pub fn parse_task_handle(src: &str) -> Option<ParsedTaskHandle> {
    // Split on the LAST separator: the number is always the final segment.
    let (prefix, number) = src.trim().rsplit_once('-')?;

    let prefix = prefix.trim();

    if number.is_empty() {
        return None;
    }

    // The prefix is held to the same rule as one being assigned to a project. Without this,
    // `RMS--5` would split into prefix `RMS-` and number 5 and "parse" into a handle whose prefix
    // can never exist — a lookup would then fail for the wrong reason, several layers away.
    if !is_valid_prefix(prefix) {
        return None;
    }

    // Digits only, checked before parsing: `"-5".parse::<i64>()` succeeds, and a negative number is
    // not a task.
    if !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let number: i64 = number.parse().ok()?;

    if number <= 0 {
        return None;
    }

    Some(ParsedTaskHandle {
        prefix: prefix.to_uppercase(),
        number,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_composed_handle_is_padded() {
        assert_eq!(compose_task_handle("RMS", 42), "RMS-000042");
        assert_eq!(compose_task_handle("RMS", 1), "RMS-000001");
    }

    /// A number wider than the padding must not be truncated — padding is a minimum, not a cap.
    #[test]
    fn a_number_wider_than_the_padding_is_kept_whole() {
        assert_eq!(compose_task_handle("RMS", 12_345_678), "RMS-12345678");
    }

    /// The point of parsing: what a human types and what the board composes are the same handle.
    #[test]
    fn padded_and_unpadded_spellings_parse_to_the_same_task() {
        let expected = ParsedTaskHandle {
            prefix: "RMS".to_string(),
            number: 42,
        };

        assert_eq!(parse_task_handle("RMS-000042"), Some(expected));
        assert_eq!(
            parse_task_handle("RMS-42"),
            Some(ParsedTaskHandle {
                prefix: "RMS".to_string(),
                number: 42,
            })
        );
    }

    #[test]
    fn parsing_upper_cases_the_prefix_and_ignores_surrounding_space() {
        assert_eq!(
            parse_task_handle("  rms-7 "),
            Some(ParsedTaskHandle {
                prefix: "RMS".to_string(),
                number: 7,
            })
        );
    }

    /// Everything that is not a handle has to come back as "not a handle", so a caller can say so
    /// rather than looking up something it invented.
    #[test]
    fn non_handles_are_refused() {
        for src in [
            "", "RMS",     // no separator
            "RMS-",    // no number
            "-42",     // no prefix
            "RMS-abc", // not a number
            "RMS--5",  // splits into prefix `RMS-`, which no project may hold
            "RMS-0",   // numbers start at 1
            "RMS-4 2", // space inside the number
            "RMS-4.2", // not an integer
            "АБВ-1",   // non-ascii prefix
        ] {
            assert_eq!(parse_task_handle(src), None, "{src:?} should not parse");
        }
    }

    /// `-` in a prefix is what would make a handle ambiguous, so it is refused where the prefix is
    /// set rather than guessed at where it is read.
    #[test]
    fn prefix_validation_refuses_what_would_make_a_handle_ambiguous() {
        assert!(is_valid_prefix("RMS"));
        assert!(is_valid_prefix("TASK_MANAGER"));
        assert!(is_valid_prefix("A1"));

        assert!(!is_valid_prefix(""));
        assert!(!is_valid_prefix("AB-C"));
        assert!(!is_valid_prefix("AB C"));
        assert!(!is_valid_prefix("АБВ")); // non-ascii
        assert!(!is_valid_prefix("THIS_PREFIX_IS_FAR_TOO_LONG"));
    }

    /// Compose and parse are each other's inverse for every valid prefix — otherwise an id shown on
    /// a sticker would not be an id anyone could type back.
    #[test]
    fn compose_and_parse_round_trip() {
        for (prefix, number) in [("RMS", 1_i64), ("TM", 999_999), ("A1", 42), ("X_Y", 7)] {
            let handle = compose_task_handle(prefix, number);
            assert_eq!(
                parse_task_handle(&handle),
                Some(ParsedTaskHandle {
                    prefix: prefix.to_string(),
                    number,
                }),
                "{handle} does not round-trip"
            );
        }
    }
}
