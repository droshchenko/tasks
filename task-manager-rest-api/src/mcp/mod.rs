use std::sync::Arc;

use mcp_server_middleware::McpMiddleware;

use crate::app::AppContext;

mod comment_tool_calls;
mod labels_list_tool_call;
mod projects_list_tool_call;
mod resolve_id_tool_call;
mod tasks_list_tool_call;
mod tasks_write_tool_calls;
mod users_list_tool_call;
mod views;

pub use views::*;

use comment_tool_calls::{AddCommentHandler, GetCommentsHandler};
use labels_list_tool_call::LabelsListHandler;
use projects_list_tool_call::ProjectsListHandler;
use resolve_id_tool_call::ResolveIdHandler;
use tasks_list_tool_call::TasksListHandler;
use tasks_write_tool_calls::{TasksCreateHandler, TasksDeleteHandler, TasksUpdateHandler};
use users_list_tool_call::UsersListHandler;

const INSTRUCTIONS: &str = "A task board that agents work and people configure. \
\
THIS IS THE ONLY WAY THE BOARD CHANGES. Projects, their columns, their kinds and who may see them are \
set up by a person in the browser; every task, every status change, every assignment and every comment \
arrives through these tools. The browser side is a viewer — nothing on it is edited with a mouse. So \
when someone says \"move that to done\" or \"put Yuri on it\" or \"note that we decided X\", there is \
no other route: it happens here or it does not happen.\
\
CALL projects_list FIRST. It is the only tool that reveals what exists, and it returns the four \
vocabularies every other call is written in: the project prefixes that name a board, the column ids a \
status must be one of, the kind ids with what each one means ON THAT BOARD, and the people who may be \
assigned work on it. Nothing here is global — two projects can have completely different columns and \
completely different kinds, so what you learned about one board tells you nothing about the next.\
\
EVERYTHING IS NAMED BY ITS HUMAN HANDLE. A project is its prefix, `RMS`. A task is `RMS-42` — or \
`RMS-000042`, which is the same id padded; both are accepted, and the padded form is what comes back. \
Never name a task by its text: two tasks may read alike, and the text is the one part of a task that \
gets rewritten.\
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
MOVING A TASK TO `done` REQUIRES A COMMENT, AND THE MOVE IS REFUSED WITHOUT ONE. Pass `comment` and \
`comment_by` to tasks_update in the same call as the status change. Say what was actually done — what \
changed, and anything the next person should know — not that it is finished, which the column already \
says. The Done column is the whole reason a board is worth reading months later, and \"moved to done\" \
records nothing anybody can use. Only the transition INTO Done needs this: a task already there can be \
re-labelled or reassigned freely. `comment` is available on any update, not just this one — it is simply \
optional everywhere else.\
\
FINISHED WORK IS NOT DELETED. It moves to `done`, which is what keeps a board readable as a history. \
tasks_delete is for a task that should never have been created, it cannot be undone, and its number is \
never reused.\
\
A NEW TASK ALWAYS STARTS IN `todo`. tasks_create takes no status — moving work on is tasks_update's \
job, which is also where landing it has to be explained. There is deliberately no way to create a task \
straight into Done.\
\
THE BOARD IS THE LAST SEVEN DAYS OF DONE, NOT ALL OF IT. Work closed more than seven days ago counts as \
archived: tasks_list leaves it out, and so does the board a person looks at. Done is the only column that \
grows for ever, and one nobody can read is one nobody looks at. Nothing is deleted — an archived task is \
still reachable by its id, and `include_archived` on tasks_list brings the history back when you are \
deliberately looking backwards. Practical consequence: \"this board has 12 tasks\" means twelve live ones, \
and a task you cannot find by listing may still exist.\
\
LIST BEFORE YOU CREATE. tasks_create adds a task unconditionally, so calling it twice for the same \
work leaves two of them on a board a person reads by eye. Check what is already there first.\
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

    mcp.register_tool_call(Arc::new(TasksListHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(ResolveIdHandler::new(app.clone())));

    mcp.register_tool_call(Arc::new(TasksCreateHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(TasksUpdateHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(TasksDeleteHandler::new(app.clone())));

    mcp.register_tool_call(Arc::new(AddCommentHandler::new(app.clone())));
    mcp.register_tool_call(Arc::new(GetCommentsHandler::new(app)));

    mcp
}
