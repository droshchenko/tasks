use std::sync::Arc;

use mcp_server_middleware::McpMiddleware;

use crate::app::AppContext;

mod comment_tool_calls;
mod documents_text_tool_calls;
mod documents_tool_calls;
mod github_git_tool_call;
mod goals_tool_calls;
mod labels_list_tool_call;
mod projects_list_tool_call;
mod resolve_id_tool_call;
mod tasks_list_tool_call;
mod tasks_search_tool_call;
mod tasks_write_tool_calls;
mod users_list_tool_call;
mod views;

pub use views::*;

use comment_tool_calls::{AddCommentHandler, GetCommentsHandler};
use documents_text_tool_calls::{
    DocumentsDiffHandler, DocumentsEditHandler, DocumentsOutlineHandler, DocumentsSearchHandler,
};
use documents_tool_calls::{
    DocumentsDeleteFolderHandler, DocumentsDeleteHandler, DocumentsGetHandler,
    DocumentsHistoryHandler, DocumentsListHandler, DocumentsRestoreHandler, DocumentsTrashHandler,
    DocumentsUpdatePathHandler, DocumentsUploadHandler,
};
use github_git_tool_call::GithubGitHandler;
use goals_tool_calls::{
    GoalsAddCommentHandler, GoalsCreateHandler, GoalsDeleteHandler, GoalsGetCommentsHandler,
    GoalsListHandler, GoalsUpdateHandler,
};
use labels_list_tool_call::LabelsListHandler;
use projects_list_tool_call::ProjectsListHandler;
use resolve_id_tool_call::ResolveIdHandler;
use tasks_list_tool_call::TasksListHandler;
use tasks_search_tool_call::TasksSearchHandler;
use tasks_write_tool_calls::{TasksCreateHandler, TasksDeleteHandler, TasksUpdateHandler};
use users_list_tool_call::UsersListHandler;

const INSTRUCTIONS: &str = "A task board that agents work and people configure. \
\
THIS IS ALL BUT ONE WAY THE BOARD CHANGES. Projects, their columns, their kinds and who may see them \
are set up by a person in the browser; every task, every assignment, every comment, every goal arrives \
through these tools. So when someone says \"put Yuri on it\" or \"note that we decided X\", there is no \
other route: it happens here or it does not happen.\
\
The exception is worth knowing about because it changes what you can assume between two calls: a person \
can DRAG A CARD BETWEEN COLUMNS in the browser, and can recolour a goal. So a task's status may have \
moved since you last read it, by a hand rather than by a tool — re-read before reasoning about where \
work sits, and treat a status you were told about minutes ago as stale. A landing done that way still \
carries its comment: the browser is made to ask for one, the same rule this tool obeys.\
\
CALL projects_list FIRST. It is the only tool that reveals what exists, and it returns the five \
vocabularies every other call is written in: the project prefixes that name a board, the column ids a \
status must be one of, the kind ids with what each one means ON THAT BOARD, the people who may be \
assigned work on it, and the goals its work is organised under. Nothing here is global — two projects \
can have completely different columns and completely different kinds, so what you learned about one \
board tells you nothing about the next. One conversation normally works on one project, so \
`projects_list` with that prefix is the call to make.\
\
WORK IS ORGANISED BY GOAL, NOT BY TASK. A goal is a container — an epic: the outcome being pursued, the \
thread where it is discussed, and the tasks that came out of that discussion. When a conversation is \
about an outcome rather than one piece of work, open a goal with goals_create, keep the reasoning on its \
thread with goals_add_comment, and create the tasks under it as they become clear — that order is the \
point, not a nicety. A goal is named by a handle like `RMS-G7`, which is what goes in `goal` on a task; \
its number comes from the same counter task numbers come from, so `RMS-7` and `RMS-G7` are never both \
real. A goal has no columns: it is open or closed. Its progress is counted from its tasks, so there is \
nothing to keep in sync — and closing it is refused until every one of those tasks is `done`, and \
requires a resolution comment, for the same reason landing a task does. Goals are never deleted. A task \
with no goal is a perfectly normal thing and not an unfinished one — but if a goal fits, put it there, \
because a board of loose tasks is a board nobody can see the shape of.\
\
EVERYTHING IS NAMED BY ITS HUMAN HANDLE. A project is its prefix, `RMS`. A task is `RMS-42` — the \
number as it reads, no leading zeros, and that is the form every tool hands back. The zero-padded \
`RMS-000042` is an older spelling of the same id and is still accepted on the way in, so an id quoted \
out of an old chat resolves; nothing produces it any more. Never name a task by its text: two tasks \
may read alike, and the text is the one part of a task that gets rewritten.\
\
AN ID FROM OUTSIDE THIS CONVERSATION IS NOT SAFE TO TRUST — USE tasks_resolve_id. A project's prefix \
can be renamed, and the freed prefix can then be taken by a different project, so `RMS-42` can mean \
one task today and a different one in the message you are reading it from. When an id comes from a \
tool call you just made, it is current and fine. When it comes from an older conversation, a commit \
message or a pasted link, resolve it first — the tool reports what the id means now AND which projects \
used to answer to it, and if there is any ambiguity you should say so before acting on it.\
\
COLUMNS ARE DATA, NOT A FIXED LIST. Every project has `todo` at one end and `done` at the other, \
always; everything between them is configured per project and has its id typed in by a person. A task \
whose stored status names a column that no longer exists reads as `todo` — the value is left alone, so \
the task returns to its column if that column is ever re-created. Practical consequence: filter and \
report using the ids projects_list gave you for THAT board, and do not assume a column called \
`in-progress` exists anywhere.\
\
KINDS ARE ALSO PER PROJECT, AND THEIR DESCRIPTIONS ARE THE POINT. `bug` on one board may be \
\"a customer-visible defect\" and on another \"anything that used to work\". Read the description \
before classifying, and if none of them fit, leave the kind off — it is optional. A task pointing at a \
kind that was deleted reads as having no kind.\
\
LABELS ARE JUST WORDS, AND THE SPELLING IS THE WHOLE CONTRACT. There are no definitions and no \
list to maintain: a label exists for exactly as long as some task wears it, a new one is created by \
using it, and the last task dropping it removes it. Which is why you must call labels_list (or read \
`labels` from projects_list) BEFORE tagging: `net-summary` and `netsummary` are two different tags, \
and once both exist every filter on either one is quietly wrong about half the board.\
\
AN ASSIGNEE IS AN EMAIL, NEVER A NAME. The board displays names and stores addresses, so a first name \
written into `assignee` names nobody at all. When someone is named in conversation — \"assign it to \
Yuri\" — call users_list and match the name to an address; pass the project so you only consider \
people who may actually see that board. If more than one person fits, ask which. Do not guess.\
\
A TASK CAN BE PUT ON AN AGENT. The literal `AI` is a valid assignee on EVERY board and means the task \
is an agent's to do rather than a person's — assign work to yourself with that, never with an address. \
It is not a user: it has no row on the roster, is never disabled, and needs no membership anywhere, \
which is why `users_list` always returns it first with `reserved` true. Neutral on purpose, so it does \
not have to change when whatever does the work does. Written in any case — `ai` reads the same as `AI`.\
\
DEPENDENCIES ORDER THE WORK, AND THEY DO NOT CROSS PROJECTS. A task lists the tasks blocking it in \
`depends_on` (ids of tasks on the same board; bare numbers work too). From that, every read derives \
`blocked` — true while any blocker is not `done`, INCLUDING an id that matches no task, so a mistyped \
or deleted blocker keeps the task blocked rather than silently freeing it. Do not start a blocked \
task, and do not move it into a working column. Nothing needs updating when a blocker lands: closing \
the last one clears `blocked` on the next read. Depending on a task that is already `done` blocks \
nothing.\
\
`blocks` IS THE HALF YOU CANNOT SEE FROM THE TASK. It is derived from the rest of the board and lists \
the tasks waiting on this one. Check it before parking, re-scoping or deleting something — \
`depends_on` tells you what is in your way, `blocks` tells you who you are in the way of, and only the \
second one is invisible unless you look.\
\
MARKDOWN IS THE FORMAT. A task's text and a comment's text are both rendered, so write them in it: \
**bold** for the headline of the work, `code` for identifiers, paths and commands, `-` bullets for a \
short checklist. Keep a task to a sticker's worth — a line or two, not a document. What does not fit \
belongs in the thread.\
\
THE THREAD IS FOR WHAT YOU LEARNED; THE TEXT IS FOR WHAT IS TO BE DONE. Findings, decisions, dead \
ends and questions go on as comments with tasks_add_comment — passing `who` (an email, or `AI`) and \
Markdown text; the moment is stamped for you. Rewrite the task's own text only when the work itself \
changed. A comment deliberately does not move the task's `updated`, so a busy thread does not read as \
active work. Every task reports `comments_amount`; when it is not zero, tasks_get_comments is worth \
reading before picking the task up — the reason the work is shaped the way it is usually lives there.\
\
PRIORITY IS WHAT DECIDES THE ORDER, AND THE ORDER IS WHAT YOU ARE GIVEN. A task and a goal each carry one \
of five values — `super-high`, `high`, `normal`, `low`, `super-low` — and every list comes back most urgent \
first, oldest first within one priority. That is the same order the board draws its columns in, so the top \
of `tasks_list` is the top of the column a person is looking at: to pick up the next thing, take the first \
one that is not `blocked`.\
\
`normal` IS THE DEFAULT AND MOST WORK SHOULD STAY THERE. The scale only says something while most tasks sit \
in the middle of it — a board where everything is `high` is a board with no priorities at all. Set one when \
the work is genuinely out of the ordinary, and ASK rather than guess when what you were told is vague: \
\"important\" and \"soon\" are not priorities. Re-ranking is a normal thing to do and takes effect on every \
open screen at once; there is no way to clear a priority, because `normal` is what having none means. A \
goal's priority is its own — it is not computed from its tasks, and a task does not inherit it.\
\
A CHECKLIST BREAKS ONE PIECE OF WORK DOWN INSIDE IT — IT IS NOT A SECOND BOARD. A task and a goal each \
carry `subtasks`: an ordered list of items, each with a one-line `title`, an optional longer `text`, and \
`done`. Write one with `add_subtasks` once you have read a task and worked out what it involves, and tick \
items with `check_subtasks` as you go — that is how the next reader sees where you got to without reading \
the whole thread. Every item is named by the `id` its checklist reports, never by its title, and an id \
that names no item is refused rather than ignored, because the alternative is reporting a tick that never \
happened.\
\
NOTHING IS DERIVED FROM A CHECKLIST, ON PURPOSE. An unticked item does not make a task `blocked`, does \
not stop it moving to `done`, and does not hold a goal open — a goal still closes on whether its TASKS \
are done. So the line is: if a step only matters to whoever is doing this one task, it is a checklist \
item; if somebody else has to see it, schedule it, be assigned it or depend on it, it is a task of its \
own under the same goal. Putting real work in a checklist hides it from the board, which is the one \
thing the board is for.\
\
A DOCUMENT IS A TEXT THE BOARD POINTS AT, NOT A LONGER TASK. Anything that outlives the work — a \
specification, a decision written up, a piece of reference — is a document: it lives at a path like \
`docs/design/system.md`, it belongs to one project, and tasks and goals REFERENCE it instead of copying \
it into their own text. That is the point: one place that is edited, rather than three copies that drift \
apart in silence. Write one with documents_upload, find one with documents_list, read one with \
documents_get, and attach it with add_documents on a task or a goal.\
\
A LARGE DOCUMENT IS WORKED ON IN PIECES, AND FOUR TOOLS EXIST FOR NOTHING ELSE. A specification can be \
tens of kilobytes; reading one whole to change a line, or reading five to find which mentions a thing, \
spends the context the work needed. So:\
\
* documents_search finds WHICH document, and on which line;\
* documents_outline turns one document into a kilobyte of headings, each with the line range of its \
section;\
* documents_get takes `from_line` / `to_line` — the outline's numbers, verbatim — and reads that \
section alone. It reports `truncated`, and a truncated read supports neither an edit nor a conclusion \
that something is not mentioned;\
* documents_edit changes the parts that change and leaves the rest untouched. Prefer it over \
documents_upload for anything you did not write in this conversation.\
\
DOCUMENTS_EDIT REFUSES RATHER THAN GUESSES, AND BOTH REFUSALS ARE THE POINT. Text that appears more \
than once is ambiguous and the whole batch fails, naming the count — quietly changing all seven \
occurrences of a word in an architecture document is how one goes wrong in a place nobody reads again. \
And `expected_version` refuses when the document moved since you read it: pass it whenever you read the \
document earlier in the conversation, because a person can upload over one from the browser at any \
moment and your edits would otherwise be spliced into a text you have never seen. Either way NOTHING is \
written — an edit batch is all of it or none of it, one new version per call.\
\
AFTER A WRITE, DIFF IT. documents_diff compares two versions and returns neither, so checking that a \
write did what you meant costs a few hundred bytes rather than two full texts. It is also what tells \
you what somebody else changed when an edit was refused on its version.\
\
THE PATH IS THE KEY; THE ID IS THE IDENTITY. Uploading to a path that is taken does not create a second \
document — it writes a new version of the one that lives there, keeping its id, its history and every \
reference to it. So documents_list BEFORE uploading: an upload to a path you did not mean to touch \
replaces what somebody put there. The id, on the other hand, never changes for as long as the document \
exists, which is why moving a document with documents_update_path breaks nothing, and why references are \
stored as ids and never as paths. Moving is a separate call from uploading on purpose — a history that \
could not tell a rewrite from a move would not answer the question it exists for.\
\
FOLDERS ARE NOT REAL. They are read off the paths of the documents in them, so an empty folder cannot \
exist and renaming one means moving every document under it, one call each.\
\
NOTHING ABOUT A DOCUMENT IS LOST. Every version is kept whole — documents_history says what happened, \
when, and who did it, for a live document and a deleted one alike. documents_delete puts a document in \
the trash rather than destroying it; documents_restore brings it back, by default exactly where it was. \
References to a deleted document are deliberately NOT cleaned up, because restoring is one call away and \
a reference quietly dropped would not come back with it. The trash is invisible in the browser: \
documents_trash is the only way to see what is in it.\
\
A CONNECTED REPOSITORY IS A REAL CLONE, AND YOU CAN WORK IN IT. A project may connect GitHub \
repositories; each appears in that project's documents as `github/<connection>/…`, and what is behind \
those paths is a working copy on this server. Every documents tool works on them: documents_list shows \
them beside the project's own, documents_get reads one, documents_search does not reach them (it \
searches this product's own texts — use `git grep` for a repository). documents_edit, documents_upload, \
documents_delete and documents_update_path WRITE them.\
\
A WRITE LANDS IN THE WORKING TREE AND STOPS THERE, WHICH IS THE WHOLE CONTRACT. Nothing is staged, \
committed or pushed by editing a file — that would put a commit on somebody's branch for every edit you \
make. Recording and sending the work is github_git, which runs any git command you like in that clone: \
`git diff` to see what you changed, `git add` and `git commit -m \"…\"` to record it, `git push` to send \
it, `git pull` to take what others did. Conflicts arrive as markers in the files, which documents_get \
shows and documents_edit fixes, then `git add` and `git commit` — or `git merge --abort` to undo the \
attempt. So: edit with the documents tools, then reach for github_git.\
\
A FILE IN A REPOSITORY HAS NO VERSION HERE, AND THAT IS NOT A GAP. documents_history, documents_diff \
and documents_restore refuse one, and say which git command answers the same question — `git log`, \
`git diff`, `git checkout`. Its history is the repository's and is complete; this product simply is not \
the thing keeping it. `expected_version` means nothing on one for the same reason: every read reports \
version 0, and `git status` is what tells you whether it moved under you.\
\
A BUILD IS RECORDED ON THE TASK THAT PRODUCED IT. When a change made in a task is built, put the \
GitHub Actions run on that task with `add_gh_actions` on tasks_update: the `url` of the run and a `title` \
saying what shipped — `my-service v1.2.3`. It is the reverse of a document: a document is what the work \
was done AGAINST, a build is what came OUT of it, and nothing else in this service records that \
connection. Record it as the build happens, ideally in the same call that lands the task; a link nobody \
attached is one somebody has to go and find in a CI history later. The same url twice is one build, the \
moment is stamped for you, and no check is made against GitHub — what is stored is what you said, so say \
it accurately.\
\
MOVING A TASK TO `done` REQUIRES A COMMENT, AND THE MOVE IS REFUSED WITHOUT ONE. Pass `comment` and \
`comment_by` to tasks_update in the same call as the status change. Say what was actually done — what \
changed, and anything the next person should know — not that it is finished, which the column already \
says. The Done column is the whole reason a board is worth reading months later, and \"moved to done\" \
records nothing anybody can use. Only the transition INTO Done needs this: a task already there can be \
re-labelled or reassigned freely. `comment` is available on any update, not just this one — it is simply \
optional everywhere else.\
\
FINISHED WORK IS NOT DELETED. It moves to `done`, which is what keeps a board readable as a history. \
tasks_delete and goals_delete are for something that should never have been created — and deleting a goal \
is not closing it: closing says how it went, deleting says it should not be there.\
\
DELETION IS A FLAG, NOT A REMOVAL. What is deleted leaves every board, list and count, and stays exactly \
where it was: searching for its id still finds it and reports it as deleted. That is the point — an id \
coming back as \"no such task\" would be indistinguishable from a typo and from another board's id. Undo \
it with `deleted: false` on tasks_update or goals_update. A number is never reused either way.\
\
A NEW TASK ALWAYS STARTS IN `todo`. tasks_create takes no status — moving work on is tasks_update's \
job, which is also where landing it has to be explained. There is deliberately no way to create a task \
straight into Done.\
\
THE BOARD IS THE LAST FEW DAYS OF DONE, NOT ALL OF IT. Work closed longer ago than the project's \
archive window — seven days unless that project says otherwise — counts as archived: tasks_list leaves \
it out, and so does the board a person looks at. A closed goal ages off the same way, on the same clock. Done is the only column that \
grows for ever, and one nobody can read is one nobody looks at. Nothing is deleted — an archived task is \
still reachable by its id, and `include_archived` on tasks_list brings the history back when you are \
deliberately looking backwards. Practical consequence: \"this board has 12 tasks\" means twelve live ones, \
and a task you cannot find by listing may still exist.\
\
LIST BEFORE YOU CREATE, AND SEARCH BEFORE YOU LIST. tasks_create adds a task unconditionally, so \
calling it twice for the same work leaves two of them on a board a person reads by eye. tasks_search \
finds a near-duplicate filed under a wording you would not have guessed, and finds it across archived \
work too — \"how did we solve this last time\" is a question about work that has already landed.\
\
THIS BOARD IS A PLACE TO LOOK THINGS UP, NOT ONLY A PLACE TO FILE THEM. Two tools search, and reaching \
for them first is what stops a conversation being reconstructed from scratch every time:\
\
* tasks_search — over tasks, goals AND their comment threads. IT IS THE ONLY WAY TO SEE INSIDE A \
THREAD without already knowing which card to open: every listing reports `comments_amount` and not one \
comment, so without this the reasoning behind every decision here is reachable only by opening threads \
one at a time. The card says what is to be done; the thread says what was learned, what was tried and \
why the work is shaped this way — and that is usually the thing you came for.\
* documents_search — over the texts of a project's documents, returning matching lines and never a \
whole document. \"Which of these twenty-three documents mentions X\" is one call; reading them to find \
out spends the context the work needed on the twenty-one that do not.\
\
Both return LINES rather than objects, on purpose: a search you can afford is a search you make before \
guessing, and one that returned whole tasks or whole documents would cost what it was meant to save.\
\
ERRORS ARE TEXT, AND THEY ARE NEVER AN EMPTY RESULT. An unknown prefix, a column that does not exist \
on this board, an id that is not there — each comes back as a message that names the real options. An \
empty list means the board really is empty, and can be reported as such.";

/// The MCP surface, mounted on the same service-sdk HTTP server as the REST controllers.
///
/// Unlike the REST side this one **writes**: it is the whole mutation surface of the product. It has no
/// authorization in this version — it sees every project and writes to any of them, closed by the
/// perimeter alone. That asymmetry (reads gated by Google sign-in and project membership, writes gated
/// by nothing) is recorded in `TODO.md` as the first thing to fix.
pub fn build_middleware(app: Arc<AppContext>) -> McpMiddleware {
    let mut mcp = McpMiddleware::new(
        "/mcp",
        crate::app::APP_NAME,
        crate::app::APP_VERSION,
        INSTRUCTIONS,
    );

    // Registered in the order they are meant to be called, which is also the order they read in a
    // client's tool list.
    mcp.register_tool_call(Arc::new(ProjectsListHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(UsersListHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(LabelsListHandler::new(app.clone())));

    mcp.register_tool_call(Arc::new(GoalsListHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(TasksListHandler::new(app.clone())));
    // Beside the listing, because it is the other half of the same act: a listing answers "what is in this
    // state", a search answers "where was this discussed", and an agent arriving at a board needs both.
    mcp.register_tool_call(Arc::new(TasksSearchHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(ResolveIdHandler::new(app.clone())));

    mcp.register_tool_call(Arc::new(GoalsCreateHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(GoalsUpdateHandler::new(app.clone())));

    mcp.register_tool_call(Arc::new(TasksCreateHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(TasksUpdateHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(TasksDeleteHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(GoalsDeleteHandler::new(app.clone())));

    // Documents. After the board tools, because a document is read in the course of doing work rather than
    // to find out what the work is — and in the order a large one is actually approached: find which
    // document, see its shape, read the part that matters, then write.
    mcp.register_tool_call(Arc::new(DocumentsListHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(DocumentsSearchHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(DocumentsOutlineHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(DocumentsGetHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(DocumentsHistoryHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(DocumentsDiffHandler::new(app.clone())));

    // The edit before the upload, which is the order they should be reached for: an upload replaces a whole
    // text, and on anything large that is the expensive way to change a sentence.
    mcp.register_tool_call(Arc::new(DocumentsEditHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(DocumentsUploadHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(DocumentsUpdatePathHandler::new(app.clone())));

    // The single deletion first, and the folder one straight after it: the second is the first repeated,
    // and a reader of this list should meet them in that order rather than discover the bulk one on its own.
    mcp.register_tool_call(Arc::new(DocumentsDeleteHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(DocumentsDeleteFolderHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(DocumentsTrashHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(DocumentsRestoreHandler::new(app.clone())));

    // After the documents tools, because that is the order the work happens in: a change is made with
    // those, and this is what records and sends it. It is also the only tool here that does not act on
    // the board at all.
    mcp.register_tool_call(Arc::new(GithubGitHandler::new(app.clone())));

    mcp.register_tool_call(Arc::new(GoalsAddCommentHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(GoalsGetCommentsHandler::new(app.clone())));

    mcp.register_tool_call(Arc::new(AddCommentHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(GetCommentsHandler::new(app)));

    mcp
}

#[cfg(test)]
mod tests {
    /// The checklist fields are the first **nested objects** on this surface — every other input is a scalar
    /// or a list of strings. Nothing in the service exercises schema generation, so a shape the derive
    /// cannot describe would first be noticed by a client asking for the tool list, which is a long way from
    /// here. Proved in a test instead, on both tools that take one.
    #[tokio::test]
    async fn the_write_tools_describe_their_checklist_fields() {
        let tasks = super::tasks_write_tool_calls::TasksUpdateInput::get_json_schema(false)
            .await
            .build();

        let goals = super::goals_tool_calls::GoalsUpdateInput::get_json_schema(false)
            .await
            .build();

        for schema in [tasks, goals] {
            for expected in [
                "add_subtasks",
                "check_subtasks",
                "uncheck_subtasks",
                "edit_subtasks",
                "remove_subtasks",
                // From the nested item itself, which is the half a flat-only schema would lose.
                "title",
            ] {
                assert!(
                    schema.contains(expected),
                    "the schema does not mention {expected}: {schema}"
                );
            }
        }
    }

    /// `edits` is the third nested object on this surface, and the one where a schema the derive cannot
    /// describe would be worst: a client that could not see `old_string` and `new_string` would send
    /// something shaped differently, and the tool would refuse every call for a reason nobody could read
    /// from the tool list. Same failure mode as the two above, one tool.
    #[tokio::test]
    async fn documents_edit_describes_its_edit_objects() {
        let schema = super::documents_text_tool_calls::DocumentsEditInput::get_json_schema(false)
            .await
            .build();

        for expected in [
            "edits",
            "expected_version",
            // From the nested item itself, which is the half a flat-only schema would lose.
            "old_string",
            "new_string",
            "replace_all",
        ] {
            assert!(
                schema.contains(expected),
                "the schema does not mention {expected}: {schema}"
            );
        }
    }

    /// The build links are the other nested object here, and unlike the checklist they are on ONE tool — a
    /// task produces builds, a goal does not — so they get their own test rather than a line in the loop
    /// above. Same failure mode either way: a shape the derive cannot describe is first noticed by a client
    /// asking for the tool list.
    #[tokio::test]
    async fn tasks_update_describes_its_build_fields() {
        let schema = super::tasks_write_tool_calls::TasksUpdateInput::get_json_schema(false)
            .await
            .build();

        for expected in [
            "add_gh_actions",
            "remove_gh_actions",
            // From the nested item's own `url` property, which is the half a flat-only schema would lose —
            // this phrase appears nowhere else.
            "actions/runs/<id>",
        ] {
            assert!(
                schema.contains(expected),
                "the schema does not mention {expected}: {schema}"
            );
        }
    }
}
