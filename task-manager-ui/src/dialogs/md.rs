/// A text somebody else wrote, as HTML.
///
/// One function rather than a `markdown::to_html` at each place text is shown, because the options are the
/// whole point and a call that forgets them looks identical to a call that does not. Everything rendered here
/// arrives the same way — an agent wrote it through `/mcp` — so it is all rendered the same way.
///
/// **GFM, not CommonMark.** `markdown::to_html` is CommonMark, where a pipe table is not a construct at all:
/// the rows are ordinary text, so consecutive lines join into one paragraph and a table of nine networks
/// arrives as a wall of `|` and `---`. That is what a task carrying a table actually looked like. GFM also
/// brings strikethrough, bare URLs as links, and `- [ ]` as a checkbox — all of which agents write.
///
/// **Still escaped.** `Options::gfm()` leaves `allow_dangerous_html` off, so raw HTML in the text is written
/// out as text instead of becoming markup. That is what makes `dangerous_inner_html` safe on the far side; do
/// not turn it on. `gfm_tagfilter` is on top of that.
pub fn md_to_html(text: &str) -> String {
    // The `Err` arm is unreachable for us: the signature carries MDX's parse errors, and MDX is off. Falling
    // back to the CommonMark render rather than unwrapping, so the worst case is a table that reads badly
    // again rather than a dialog that renders nothing.
    markdown::to_html_with_options(text, &markdown::Options::gfm())
        .unwrap_or_else(|_| markdown::to_html(text))
}
