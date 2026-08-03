use dioxus::prelude::*;

/// Confirm reading a connected repository from GitHub again.
///
/// **A confirmation for a read looks like ceremony and is not.** This is the one gesture on the
/// Documents screen that spends somebody else's budget: every listing is a request against an hourly
/// allowance shared by the whole project — sixty an hour without a key — and a button that re-read the
/// repository on every stray click would exhaust it for everybody reading files at the same time.
///
/// It says what will happen rather than asking "are you sure", because the useful thing to know is that
/// the answer does not arrive with the button: the reading happens in the background, and the tree shows
/// it when it is asked again.
#[component]
pub fn PullGithubDialog(connection: String, on_submit: EventHandler<()>) -> Element {
    let feedback = super::feedback();

    let content = rsx! {
        div { class: "confirm-body",
            p {
                "Read "
                span { class: "mono", "{connection}" }
                " from GitHub again, rather than waiting for the next ten-minute check."
            }
            p { class: "field-hint",
                "Only the file list is read — the files themselves are fetched when you open them. It happens in the background, so press Refresh in a moment to see the result."
            }

            if !feedback.error.is_empty() {
                div { class: "error-note", "{feedback.error}" }
            }
        }
    };

    let ok = rsx! {
        button {
            class: "btn btn-primary",
            disabled: feedback.saving,
            onclick: move |_| on_submit.call(()),
            if feedback.saving { "Asking…" } else { "Read again" }
        }
    };

    super::dialog_template("Refresh the repository", content, ok)
}
