//! A task's title, read out of its text.
//!
//! A task has no title field — agents write one body, and its first line is the sentence they lead with. So
//! the title is derived rather than stored, which means nothing has to be migrated and a task written before
//! anything asked for a title still has one.
//!
//! Here rather than in the UI because two places now need the same answer — the card on the board and the
//! header of the dialog behind it — and a title that differed between them would read as two tasks.

/// The first line of the text, with the Markdown it is wrapped in taken off.
///
/// A heading's `#`, a bullet's dash, a quote's `>`, and emphasis around the whole line. What comes back is
/// plain text meant to be shown as one line, never rendered.
pub fn task_title(text: &str) -> String {
    for line in text.lines() {
        let line = strip_markdown_markers(line.trim());

        if !line.is_empty() {
            return line.to_string();
        }
    }

    "(no text)".to_string()
}

fn strip_markdown_markers(line: &str) -> &str {
    let mut line = line;

    // Repeated, because a line can carry more than one of these at once — `> ## Title` is a heading inside
    // a quote, and both have to come off before the text starts.
    loop {
        let stripped = strip_one_marker(line);

        if stripped == line {
            break;
        }

        line = stripped;
    }

    // Emphasis wrapping the WHOLE line only. Trimming these characters anywhere would eat the underscores
    // out of a crate name and the backticks off an identifier that is only part of the sentence.
    for marker in ["**", "__", "*", "_", "`"] {
        if line.len() > marker.len() * 2 && line.starts_with(marker) && line.ends_with(marker) {
            line = &line[marker.len()..line.len() - marker.len()];
            break;
        }
    }

    line.trim()
}

/// One block marker off the front, or the line unchanged.
///
/// A bullet is only a bullet when whitespace follows it — otherwise `**Ship it**` would lose its opening
/// asterisks to the list rule and never reach the emphasis rule.
fn strip_one_marker(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix('>') {
        return rest.trim_start();
    }

    let hashes = line.chars().take_while(|c| *c == '#').count();

    if hashes > 0 {
        let rest = &line[hashes..];

        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return rest.trim_start();
        }
    }

    for bullet in ['-', '*', '+'] {
        if let Some(rest) = line.strip_prefix(bullet) {
            if rest.starts_with(char::is_whitespace) {
                return rest.trim_start();
            }
        }
    }

    line
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card's whole face comes out of this, so what it does to a first line is worth pinning.
    #[test]
    fn the_title_is_the_first_line_of_the_text() {
        assert_eq!(
            task_title("Fix the login redirect\n\nIt drops the return url."),
            "Fix the login redirect"
        );

        assert_eq!(
            task_title("\n\n  Fix the login redirect  \n"),
            "Fix the login redirect",
            "leading blank lines are skipped and the line is trimmed"
        );

        assert_eq!(task_title(""), "(no text)");
        assert_eq!(task_title("\n \n"), "(no text)");
    }

    /// Agents write Markdown, so the first line arrives wrapped in whatever they lead with.
    #[test]
    fn the_title_comes_out_of_its_markdown() {
        assert_eq!(
            task_title("# Fix the login redirect"),
            "Fix the login redirect"
        );
        assert_eq!(task_title("### Fix it"), "Fix it");
        assert_eq!(task_title("- Fix it"), "Fix it");
        assert_eq!(task_title("* Fix it"), "Fix it");
        assert_eq!(task_title("> ## Fix it"), "Fix it", "both markers come off");
        assert_eq!(task_title("**Fix it**"), "Fix it");
        assert_eq!(task_title("`Fix it`"), "Fix it");
    }

    /// The characters emphasis is made of also appear inside ordinary text, and taking them out anywhere
    /// would mangle a crate name or a multiplication.
    #[test]
    fn the_title_keeps_markers_that_are_not_markers() {
        assert_eq!(
            task_title("Rename my_jet_tools to something else"),
            "Rename my_jet_tools to something else"
        );
        assert_eq!(
            task_title("*emphasis* only at the start is not a wrapper"),
            "*emphasis* only at the start is not a wrapper"
        );
        assert_eq!(
            task_title("Publish task-manager-ui 0.1.13"),
            "Publish task-manager-ui 0.1.13",
            "a dash inside a word is not a bullet"
        );
    }
}
