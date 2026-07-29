//! How a task handle is SHOWN.
//!
//! A task is stored and travels padded — `RMS-000042` — and that is not up for negotiation: the padded form
//! is what the MCP tools hand back and what an id sorts correctly as. But nobody says "RMS zero zero zero
//! zero forty-two", and a column of six-digit handles is six characters of nothing repeated down the board.
//! So the padding is a storage detail, and this is the one place that takes it off.
//!
//! Here rather than in the UI because the same answer is needed in four places — the card, the dialog header,
//! the dependency lists and their tooltips — and a handle that differed between them would read as two tasks.
//!
//! It is display only. Nothing parses what comes out of here, and nothing sends it anywhere: a lookup, a
//! link, a `key` on a list all keep the id exactly as it arrived.

/// A task handle with its padding taken off: `RMS-000042` → `RMS-42`.
///
/// Anything that is not a handle comes back untouched. That matters more than it looks: this runs on whatever
/// the server put in `id`, and the day something arrives in a shape nobody predicted, showing it verbatim is
/// the answer that loses nothing.
pub fn task_id_display(id: &str) -> String {
    let Some((prefix, number)) = id.rsplit_once('-') else {
        return id.to_string();
    };

    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
        return id.to_string();
    }

    // A number that is all zeros keeps one digit. `RMS-` alone would not read as a handle at all, and zero is
    // not a task anyway — this only decides which of two wrong-looking things to show if it ever happens.
    let trimmed = number.trim_start_matches('0');
    let trimmed = if trimmed.is_empty() { "0" } else { trimmed };

    format!("{prefix}-{trimmed}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_padding_comes_off() {
        assert_eq!(task_id_display("RMS-000042"), "RMS-42");
        assert_eq!(task_id_display("RMS-000001"), "RMS-1");
        assert_eq!(task_id_display("RMS-123456"), "RMS-123456");
        assert_eq!(
            task_id_display("TM_2-000007"),
            "TM_2-7",
            "an underscore is legal in a prefix"
        );
    }

    #[test]
    fn a_handle_that_needs_nothing_done_to_it_is_left_alone() {
        assert_eq!(task_id_display("RMS-42"), "RMS-42");
    }

    /// Whatever is in `id` gets shown, so anything unexpected has to survive the trip rather than come out
    /// mangled or empty.
    #[test]
    fn anything_that_is_not_a_handle_is_shown_as_it_is() {
        assert_eq!(task_id_display(""), "");
        assert_eq!(task_id_display("RMS"), "RMS");
        assert_eq!(task_id_display("RMS-"), "RMS-");
        assert_eq!(task_id_display("RMS-4a2"), "RMS-4a2");
        assert_eq!(task_id_display("RMS-000000"), "RMS-0", "zero keeps a digit");
    }
}
