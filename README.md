# task-manager-mcp

A task board that **agents work and humans configure**. Every task mutation arrives through MCP,
with two named exceptions the browser owns: **moving a card between columns**, and **a goal's
colour**. Nothing else is edited with a mouse — not a task's text, not its type, not who is on it,
not a thread.

That split is the whole design. It is not a Jira with an MCP bolted on — the MCP surface is the
primary interface, and the UI exists because projects, columns, kinds and people have to be
configured by a person, and because someone wants to see the board.

The two exceptions are deliberate and each is one field wide. A colour is presentation rather than
state, and task types have always been coloured with a mouse. A move is the one gesture a board is
expected to have; refusing it taught people the screen was broken rather than that it was a viewer.
Both go through the same scripts and the same validation an MCP call does — a landing in `done`
still owes a comment — and the move signs that comment with the session, which is the one thing this
door does better than MCP, where the author is a string the caller passes.

Product namespace on the host: `task-manager-mcp`. Successor to `rms/development-tasks-mcp`, which
this replaces once the first version lands.

## Two services

| | |
|---|---|
| **`task-manager-rest-api`** | service-sdk HTTP. Three surfaces on one port: `/api/v1/*` (reads for the UI, configuration CRUD, and the two writes the board owns — a task's column and a goal's colour), `/mcp` (every other task mutation), `/ws` (the whole board, pushed). Owns Postgres. |
| **`task-manager-ui`** | Dioxus CSR (`dioxus/web`), a static bundle. Talks to the REST API through `flurl`. |
| **`task-manager-shared`** | Wire models, shared verbatim by both. WASM-clean by default; a `server` feature gates `MyHttpInput` / `MyHttpObjectStructure`. |

**Deliberate deviation from `architect-playbook`.** By the archetype table an internal employee
admin panel is `admin-ui` (Dioxus fullstack, server functions in the deployable), and the
`rest-api` + CSR-`ui` pair is reserved for external `client-ui`. We take the pair anyway, and
`task-manager-rest-api` owns Postgres directly rather than fronting a `grpc-flows` domain-owner.
Reason: there is exactly one consumer plus the MCP surface, both in the same process, and
`rms/dashboards-rest-api` + `dashboards-ui` is the same shape already in production. Splitting a
single responsibility by transport would buy a proto file and two layers of mappers and nothing
else.

## Everything lives in memory

Postgres is where the data is **durable**. Memory is where it is **read**. On startup the service
loads all of it — projects, columns, kinds, tasks, comments, users, membership — builds its
indexes, and from then on no read touches the database.

**`scripts/` is the only write path, and the order inside it is fixed: Postgres first, then
memory.** If the database write fails, memory is left alone. The reverse order would produce a
state that exists in the process and not on disk — and it would survive right up to the next
restart, where it would silently roll back.

What this buys, beyond speed: the id counter has no race (it is a field under a lock, not
`MAX(number)+1`), prefix uniqueness and the `RMS-42` → project resolution are in-process lookups,
and `blocked` / `blocks` / the label set are derived on every read for free.

**There is one exception, and it is narrower than it sounds: a document's PAYLOAD.** Everything *about* a
document — its path, size, kind and id — is held in memory like everything else, in its own collection beside
the board. What is not is the bytes: a document can be a PDF, and the board is pushed *whole* down a WebSocket
on every change, so caching payloads would ship every file on the board to every open screen every time
anybody moved a sticker. Payloads are read from Postgres one document at a time, when somebody opens one. See
**Documents** below.

What it costs: **`task-manager-rest-api` is single-instance.** State is authoritative in the
process and the WebSocket fan-out is in-process, so a second replica would both diverge and fail
to notify. This is the playbook default for state-bearing services; here it is a constraint, not
a default.

## Project

The unit of everything. A task cannot exist outside one.

| Field | |
|---|---|
| `id` | `rust_extensions::SortableId` — `{unix_micros}-{uuid}` truncated to 25 chars. Chronologically sortable, never shown to a user. |
| `name`, `description` | Free text. Editable. |
| `prefix` | The human-facing half of a task id: `RMS` → `RMS-42`. Renameable. |
| `prefix_history` | Every prefix this project has ever had. |
| `columns` | Ordered. See below. |
| `kinds` | See below. |
| `members` | Users with access. Edited here, from the project's side. |

**Two projects may not hold the same prefix at the same time** — the second is refused. A prefix is
renameable, and one that has been renamed away from is free for another project to take. Every
prefix a project ever carried is kept, so `RMS-42` still resolves after a rename: the resolver looks
for a project holding `RMS` **now**, and only then falls back to prefix history.

**Which is why a task's human id is not stored.** A per-project counter plus a reusable prefix means
two projects can both produce `RMS-1` — project A while it held `RMS`, project B after taking it
over. Materialising that string would put two different tasks under one id. So the row holds only
`number`, and `RMS-42` is composed on read from the project's *current* prefix. Nothing can
collide, "the current holder wins" is automatic rather than a rule to enforce, and the cost is
explicit: after a rename A's tasks read as `TM-1`, and the old `RMS-1` link dies at the moment
another project takes `RMS` over.

For the same reason **`depends_on` holds task numbers, not id strings** — a rename would otherwise
break every dependency in the project at once.

Deleting a project is not implemented — see `TODO.md`, including why prefix history does not
protect against it.

### Columns

`todo` (leftmost) and `done` (rightmost) exist in every project and are not configurable.
Everything between them is: the user enters the column's id by hand, plus a name, a description
and its position in the order.

- **A column id is never renamed.** Only deleted.
- **A column is deleted freely**, whatever is in it.
- **A task whose status matches no column of its project reads as `todo`.**

That last rule is why there is **no foreign key** from a task's status to a column: an orphaned
status has to survive, not be rejected by the database. Side effect worth knowing — the stored
status is untouched, so re-creating a column with the same id brings its tasks back to it.

### Kinds — "task types" on screen

Same shape as columns — id entered by hand, name, description — plus a **colour from a preset set** (an
enum in `task-manager-shared`, validated by the server) and an optional **icon**. Both are drawn by the UI
as clickable choices rather than dropdowns of names, because a picker that shows the thing beats one that
spells it. Unlike columns, kinds have no mandatory anchors: a project may have none.

The icons are SVG files in `task-manager-ui/public/assets/images/task-icons`, and the list the picker
offers is **generated by `build.rs` from that directory** — dropping a file in and rebuilding is the whole
job. A kind stores only the file's stem (`bug`, `tech-debt`), and the server never validates it against a
list: the files ship in the UI bundle, so an unknown name draws as no icon, the same leniency an unknown
colour or column id gets. On a tag the icon is painted white with `brightness(0) invert(1)`, so one icon
set reads correctly on all eight tag colours without editing any file's fill.

The wire name stays `kind` everywhere — the API, the models, the MCP tools — while the screen says "task
type". Deliberate: `kind` is a contract with every agent already calling `tasks_create` with it.

### Labels

Per project, and nothing more than a name — no description, no rules, no meanings. A task picks
an existing one or introduces a new one by typing it. Consequently there is no label table: a
project's label set is the distinct labels in use across its tasks, and a label stops existing
when the last task drops it.

This is a deliberate simplification of `development-tasks-mcp`, where labels were a shared
vocabulary with written definitions, a `used_by` count and an "in use but undefined" report. That
machinery is gone.

### The reserved assignee `AI`

One assignee is not a person and has no user row: the literal `AI`, meaning the task is an agent's to do.
Neutral on purpose, so it does not have to be renamed when whatever does the work changes.

Deliberately not a row on the roster — a row would have to be added to every project's membership to be
assignable, and disabling it would read as disabling a colleague. It is assignable on every board, always,
and `users_list` returns it first with `reserved` true, which is how an agent learns it may put work on
itself.

Stored exactly as declared whatever case it arrives in, while an address is lower-cased. One function
(`normalise_actor`) does both for an assignee and for a comment's author, because a comment signed `AI` and
a task assigned `AI` have to be the same string or a filter on one would miss the other.

## Task

| Field | |
|---|---|
| `project_id` | Owner. |
| `number` | Sequential within the project, from the project's own counter. Not reused after a delete. |
| — | The human id `PREFIX-42` is **composed on read**, never stored — see above. Not padded: the number as it reads. The zero-padded `PREFIX-000042` this board used to hand out still parses, so an id quoted from an old chat or link resolves. |
| `text` | Markdown — the UI renders it. |
| `status` | A column id of this project. Unknown → reads as `todo`. **Always `todo` on creation** — `tasks_create` takes no status. |
| `priority` | One of five: `super-high`, `high`, `normal`, `low`, `super-low`. Decides where the card sits in its column. See below. |
| `kind` | A kind id of this project. Optional. |
| `assignee` | An email, or the reserved `AI`. |
| `labels` | Free tags, lowercased and de-duplicated. |
| `depends_on` | **Numbers** of blocking tasks, within the same project. |
| `subtasks` | A checklist. Each item: a `SortableId`, a one-line `title`, a longer Markdown `text`, and `done`. See below. |
| `gh_actions` | The builds this work produced. Each one: the `url` of a GitHub Actions run, a `title`, and when it was attached. See below. |
| `comments` | A thread. Each comment: moment, `who`, Markdown text. |
| `close_moment` | When the task landed in Done; absent whenever it is not there. Cleared on re-open, so a re-closed task is dated by its latest close. |
| `created`, `updated` | A comment does not move `updated` — the thread is a separate record from the work. |

**Landing work has to say what was done.** Moving a task into Done without a `comment` is refused — the
Done column is the whole reason a board is worth reading months later, and "moved to done" records nothing
anybody can use. Only the *transition* is gated: a task already there can be re-labelled or reassigned
freely. And since `tasks_create` cannot set a status, creating a task straight into Done is not a way
around it.

**The board is the last seven days of Done, not all of it.** Work closed longer ago than that counts as
archived: it is left out of the board and out of `tasks_list`. Done is the only column that grows for
ever, and one nobody can read is one nobody looks at. Nothing is deleted — an archived task is still
reachable by its id, and `include_archived` brings the history back. The window is measured from
`close_moment`, not `updated`, so editing an old finished task does not drag it back onto the board.

**`blocked` and `blocks` are derived, never stored.** `blocked` is true while any id in
`depends_on` names a task that is not `done`; an id matching no task counts as still-blocking, so
a typo or a deleted blocker does not silently free the task. `blocks` is the reverse edge, read
off the rest of the board — the only way to see who is waiting on you. Closing the last blocker
clears `blocked` on the next read, with nothing to update by hand.

## Priority — five steps, and it decides the order

A task **and a goal** each carry one of `super-high`, `high`, `normal`, `low`, `super-low`. A fixed scale
rather than a number: a number invites 7-vs-8 arguments and drifts upward until everything is a 9, whereas
five named steps make somebody choose between *this* and *that*. Unlike columns and kinds it is **not**
per-project — urgency does not mean something different on another board, and one scale is what lets two
boards be read side by side.

**The order is the whole feature.** Every list the server hands out — the board read, the WebSocket push,
`tasks_list`, `goals_list`, a goal's own task list — comes back **most urgent first, oldest first within one
priority**. Sorted in `BoardInner` rather than by each reader, so the board, the Goals screen and every agent
agree about what is at the top of a column, and a client that only groups the list into columns is already
right. Within one priority the tiebreaker is the task number, which is the order the board had before
priorities existed — so nothing moved for work nobody has ranked.

On the board, priority **outranks the goal grouping**: a Super High card is at the top of its column, not at
the top of its own group. The grouping (cards under a goal above the loose pile, cards of one goal adjacent)
is what breaks a tie between two cards ranked the same. The alternative would make Super High mean "near the
top of its own group", which is not a priority at all.

`normal` is the default, and a row written before the field existed reads as `normal` — as does a value this
build does not recognise, the same leniency a colour gets. **A write is the opposite: an unreadable priority
is refused**, naming the five, because a write is somebody deciding and quietly turning a typo into Normal is
how work nobody meant to deprioritise sinks. Spelling is lenient in both directions (`Super High`,
`super_high`, `superhigh`); the refusal is for values that mean nothing.

There is no way to clear it. `normal` *is* the absence of a ranking, so a field that could also be empty would
have two spellings for one state.

**A card shows the badge only when the priority is not Normal** — the position in the column already says it,
and a tag repeated on every card is a tag nobody reads. The two dialogs show it always, Normal included: that
is where a fact is looked up, and a row that vanishes is ambiguous where one saying "Normal" is not.

A goal's priority is **its own**, not derived from its tasks: an epic can be urgent while none of its work has
started, and a number computed from the work would take the decision away from the person making it. Nor does
a task inherit its goal's.

## The checklist — subtasks that never leave the task

A task **and a goal** each carry an ordered list of checklist items: an id, a one-line `title`, a longer
Markdown `text`, and `done`. It lives in a `jsonb` column on the row it belongs to, exactly like the
thread and for the same reason — ticking an item is then one atomic upsert of one row.

**Nothing is derived from it, and that is the whole design.** An unticked item does not make a task
`blocked`, does not stop it moving to `done`, and does not hold a goal open — a goal still closes on
whether its *tasks* are done. So a checklist somebody abandoned half-way is not a state the board has to
have an opinion about, and the rule an agent has to learn is one line: if a step only matters to whoever is
doing this one task, it is a checklist item; if somebody else has to see it, schedule it, be assigned it or
depend on it, it is a task of its own under the same goal. The failure this prevents is real work hidden
inside a card, which is the one thing a board exists to stop.

Which is also why it is **not** folded into a goal's `done_amount` / `tasks_amount`. Those count tasks and
are what decides whether the goal may close; mixing a private breakdown into them would make one counter
mean two things.

**An item is named by its id, never by its title** — two items may read alike, and a title is the part that
gets rewritten. The id is a `SortableId` minted server-side, is not a task handle, resolves nowhere else,
and is never shown to a person: the screen draws the title. It is not reused after a removal, so an id
quoted from a stale read names nothing rather than the wrong item.

**Edits are operations, not a new list.** `tasks_update` / `goals_update` take `add_subtasks`,
`edit_subtasks`, `check_subtasks`, `uncheck_subtasks` and `remove_subtasks`, applied in that order — the
same shape and the same reason as `add_labels` / `remove_labels`. A whole-list write would silently drop
whatever the caller did not know about, which on a list two people are adding to is a lost item nobody
notices. An id naming no item is **refused**, unlike a label that is not there: an unknown id means the
caller is working from a read that has moved on, and quietly doing nothing would report a tick that never
happened.

Ticking an item **does** move `updated` — a checklist is the work, not the conversation about it, which is
what separates it from a comment.

In the browser it is read-only, like everything else about a task: the dialog draws the titles under the
text, greys out what is done and marks it with a tick, and a click expands one item's `text` (an item with
no text is drawn as a plain row, so there is no affordance for an expansion that would show nothing).

**Both jsonb columns are nullable**, and not because "no checklist" needs its own spelling — an empty array
says that perfectly well. The columns arrive on tables that already have rows, and the schema generator
derives a column's nullability from the Rust type: a non-`Option` field would emit
`alter table … add subtasks jsonb not null`, which Postgres refuses on a populated table. `NULL` reads as an
empty checklist and every write puts a real array in. Same arrangement, and same reason, as `goals.color`.

## Builds — the link from the work to what shipped

A task carries `gh_actions`: the GitHub Actions runs that came out of it, oldest first. Each entry is a
run `url`, a `title` — `my-service v1.2.3` — and the moment the link was attached. It lives in a `jsonb`
column on the task row, exactly like the thread and the checklist, so recording a build is one atomic
upsert of one row and the links ride along in the board snapshot with nothing to resolve.

**It is a document reference pointing the other way, and that is why the two are drawn next to each
other.** A document is what the work was done *against*; a build is what came *out* of it. Nothing else
in this service connects a piece of work to the artefact it produced, and the question it answers —
"this change, did it ship, and as what" — is asked long after the task is closed, when the CI history is
the only other place to look and nobody remembers which run it was.

**The url is the identity.** A run already has an id and it is in the url, so nothing is minted here:
adding a url the task already carries is *not* a duplicate and not an error — the same build re-reported
is what a re-run looks like from here. The existing entry keeps its moment, because that is when this
build was recorded against the work, and takes the new title if one was given, since a caller bothering
to name it a second time is correcting the first. Removing names the url; one that is not there is not
an error, exactly as with a label.

**Nothing is fetched.** This service never talks to GitHub — what is stored is what the caller said. So
there is no run status here, nothing goes stale, and the moment is when the link was *attached* rather
than when GitHub ran anything: a time read off a url nobody fetched would be a guess dressed as a
record. For the same reason the host is not checked against `github.com` — an enterprise install answers
on its own domain, and refusing a real build over its hostname would be a cosmetic rule with a real cost.
What *is* checked is that the value is an `http(s)` link at all: a bare run number stored here would draw
a row nobody can click.

**A title is optional and never empty.** When nobody writes one, it is worked out from the url —
`…/<owner>/<repo>/actions/runs/<id>` becomes `my-service #18423`, and anything else falls back to the
host and the last segment. Requiring one would only have produced names copied out of the url by
whoever was in a hurry; a derived name says the same thing and is honest about being derived.

**Only `tasks_update` writes them, through `add_gh_actions` / `remove_gh_actions`** — the same add/remove
shape as labels and document references, and for the same reason: a task collects builds one at a time
over the life of the work, so a whole-list write would drop whatever the caller had not read.
`tasks_create` deliberately takes none: a build is what comes out of the work, so it cannot exist before
the task that produced it.

In the browser the task dialog draws them under the documents, one per line: the title as a link, the
date beside it, the url as the tooltip. They are the one thing on that screen that leaves the product, so
they open in a new tab — the board stays where it was, and the back button is not the way home from a CI
log. Most tasks produced no builds and draw nothing at all rather than an empty heading.

## Searching the board

Every other read here is a **filter**: a column, a kind, an assignee, a goal. Those answer "what is in this
state", which is what a board is for — and none of them answers the question an agent actually arrives
with, which is *"where did we discuss this"*. The text is the only thing that knows, and most of it lives on
threads that no listing returns: `tasks_list` reports `comments_amount` and not one comment, for exactly
the context reason `documents_list` reports no texts.

So without `tasks_search` the reasoning behind every decision on this board is reachable only by listing it
and then opening threads one call at a time until the right one turns up. It searches a task's text, a
goal's name and description, both checklists and **both comment threads**, and each match says which of
those it came from — because the distinction is what the caller does next with it. The card says what is to
be done; the thread says what was learned, what was tried and why the work is shaped this way.

It returns a **headline and the matching lines**, never whole tasks. A search that cost what a listing
costs would not be reached for before guessing, which is the only time it is worth anything. Results are
sorted **most-matched first** rather than by priority: a card mentioning the subject six times is more
likely the one that was meant than one mentioning it in passing.

Served entirely from memory — comments ride inside the task's own row and are already in the snapshot, so
searching the threads is a walk over what is loaded rather than a query. Archived and deleted work is left
out by default, matching `tasks_list`, so "search finds a task the board does not show" cannot happen by
accident; `include_archived` is worth passing more often here than on a listing, because "how did we solve
this last time" is a question about work that has already landed.

## Documents

A **document** is a text that outlives the work: a specification, a decision written up, a piece of
reference. It belongs to one project, it lives at a path — `docs/design/system.md` — and tasks and goals
**reference** it instead of copying it into their own text. That is the whole point: one place that gets
edited, rather than three copies that drift apart in silence.

**A document is text OR bytes.** A specification in Markdown and an uploaded PDF are both documents at both
paths, and exactly one of two nullable columns holds each: `content` for text, `binary_content` — a `bytea` —
for a file. Not an enum, because a column cannot be one; the invariant is established on the write path and
every read decides which kind it is holding by asking which column is filled. `content_type` sits beside them
as metadata (worked out from the extension when nobody says), and `content_size` is stored rather than computed
— a listing wants a size, and computing one means reading the payload.

**base64 appears in exactly one place: an MCP argument.** JSON cannot carry bytes, so `documents_upload` takes
`binary_base64` and `documents_get` hands one back, decoded and encoded at that boundary and nowhere else. The
browser never sees base64 at all — it points an `<img>` or an `<iframe>` at `/raw/{prefix}/{path}`, which serves
the real bytes with the document's own `Content-Type`, so a PDF opens in the browser's own viewer.

**That route is a PATH mirroring the tree, not a query, and that is what makes a framed html page work.** A page
asks for its own `style.css` with a relative url, which the browser resolves against the address the page came
from: served as `/raw/TM/docs/page.html`, `style.css` beside it resolves to `/raw/TM/docs/style.css` and
arrives. From a query url it would resolve back onto the api route and arrive as nothing. A path of arbitrary
depth is not something the routing macro can express, which is why this one is a middleware.

**The path is the key; the id is the identity.** `documents_upload` takes a project, a path and a payload, and
looks the path up: found, and it writes a *new version* of the document that lives there, keeping its id and
its whole history; not found, and it creates one. So "upload the file again" is the entire editing story, and
a caller holding a file and a path needs to know nothing else. The id, by contrast, is a `SortableId` minted
once and never changed — not by a rewrite, not by a move — which is what makes the history complete and what
makes a reference survive somebody tidying the tree.

Which is why **moving is a separate call**, `documents_update_path`. If an upload could carry a new path
*and* new text, the history could not tell "somebody rewrote it" from "somebody moved it", and those are the
two questions a history exists to answer.

**Folders are not real.** They are read off the paths of the documents in them, on both sides — the server
sorts the index by path, the browser walks the segments into a tree. So an empty folder cannot exist, and
renaming one means moving every document under it, one call each. A folder record would have been a second
source of truth about the structure, and it would have drifted from the paths.

### A large document is worked on in pieces

Everything above is complete **at one size** — the size where reproducing a document verbatim to change a
line is affordable, and where reading one to find out whether it mentions something is affordable. Past
that, the surface quietly becomes read-only in practice: `platform-architecture.md` at 76 KB does not fit
in a tool result at all, so it could be read and never written back, and finding which of twenty-three
documents mentions `min_statistics` meant pulling each of them in whole. Two of them, 24 KB and 33 KB,
turned out to be clean — about fifteen thousand tokens spent learning that nothing was there.

Four tools exist for nothing but that, and they compose in one direction:

**`documents_search`** asks the whole project at once and never returns a text — only matching lines and a
couple of lines either side. Line-oriented like grep, so a match carries a `line_number` that goes straight
into a read. The texts come from Postgres (the index deliberately holds no payloads) through a select list
that **does not name the blob column at all**, so a project holding forty PDFs costs the same to search as
one holding none. `matches_total` keeps counting past every cap, which is what makes "nothing matched" a
usable answer rather than a possibly-truncated one.

**`documents_outline`** turns a 76 KB document into about a kilobyte of headings, each with the line range
of the section it opens. Headings inside fenced code blocks are skipped — a specification full of shell
examples has a `# comment` on nearly every page, and an outline naming those would be longer than the real
structure and would point at sections that do not exist. A section **contains its subsections**
(`end_line` runs to the next heading at the same level or shallower), so slicing one entry gives the whole
section rather than its first paragraph.

**`documents_get` takes `from_line` / `to_line` / `max_bytes`** — the outline's numbers verbatim. Bounds
are 1-based and inclusive, `max_bytes` cuts at a line boundary wherever one fits, and `truncated` says the
content is not the whole document. A file is *refused* rather than sliced: lines are a property of text,
and half a PDF is not a smaller PDF — cutting one would hand back base64 that decodes to a corrupt file,
which is worse than a refusal because it looks like it worked.

**`documents_edit`** sends the two hundred bytes that change instead of the seventy-six kilobytes that do
not. Three rules carry the weight, and each is a refusal:

- **an ambiguous match is refused.** If `old_string` appears more than once and `replace_all` was not
  asked for, the call fails and names the count. Silently taking the first occurrence, or silently taking
  all of them, is exactly how a 76 KB architecture document ends up subtly wrong in a place nobody reads
  again;
- **all of the edits or none of them.** The new text is built in memory and only then written, so a
  failure at edit five leaves nothing behind from edits one to four — a half-applied batch is a document
  in a state nobody asked for, and the caller cannot tell how far it got without reading the whole thing
  back. Edits apply **in order, each against the result of the last**, so a later edit may match text an
  earlier one wrote, and one whose match an earlier edit destroyed is an error rather than a skip;
- **`expected_version` is an optimistic lock** — the only concurrency control in this feature, and it
  earns its place because a person can upload over a document from the browser at any moment. A caller
  that read version 4 and now edits had better still be editing version 4: the edits would very likely
  still *apply*, and would land a document mixing two intentions.

One new version per **call**, not per edit: seven edits are one history entry, because seven entries would
describe seven documents nobody ever intended. The event is `updated`, indistinguishable from an upload of
the same text — how a version was produced is a property of the call, not of the document.

**`documents_diff`** answers "did that write do what I meant" without either text. `documents_history` says
a version exists, who wrote it and when; it cannot say what is *in* it. It is also what tells you what
somebody else changed after an edit was refused on its version — which is why the refusal message names the
diff call to make. `to_version` defaults to the newest version taken from the *history* rather than from
the document row, so it answers for a trashed document exactly as it does for a live one.

The whole loop, then: search to find which document, outline to find the section, get with a line range to
read it, edit to change it, diff to check the change is the one that was meant. None of the five steps
carries a whole document.

**Nothing is lost.** Every version is kept whole in `documents_history` — path, text, who, when, and *what
happened* (`created`, `updated`, `moved`, `deleted`, `restored`). Whole texts rather than diffs: documents are
read one at a time, so nothing has to walk a chain, and a chain of diffs is a structure whose failure mode is
"the oldest version is unreadable" — the one thing this table exists to prevent. It answers for a deleted
document too, because the id outliving the deletion is the point.

**History is a second table, and the only place in this service where one change writes two rows.**
Everywhere else state rides on its own row precisely to avoid that. There is therefore no transaction to hide
behind, so the order is fixed: **history first, then the working table.** The failure that leaves behind is a
version nothing is serving — one row too many, which the next attempt overwrites, since history is *upserted*
rather than inserted. The other order loses a version outright, and a history with a hole in it is not a
history.

**Deleting is a move to a second table, not a flag.** `documents_delete` takes the row out of `documents` and
puts it in `documents_trash`; `documents_restore` brings it back, by default exactly where it was, and is
*refused* when something has taken that path since — where a restored document lands is a decision, and a
guessed path is how a document ends up somewhere nobody looks. A second table rather than a `deleted` column
because with a flag every read of the working set — the index, the path lookup, the uniqueness check — would
have to remember to exclude the deleted, and the one that forgot would be a bug nobody notices until a path
that looks free refuses a document.

The trash is **flat** (it keeps only the last path each document had) and **invisible in the browser**:
`documents_trash` is the only way to see it. It is not a place to browse — it is a list you ask for when
something needs restoring.

**A reference is a list of ids in a `jsonb` column** on the task or goal row — `add_documents` /
`remove_documents`, the same shape as labels and for the same reason: a caller attaching one document must
not have to resend the four already there. Ids and never paths, which is the whole reason the id is stable.
Two departures from how labels behave:

- an id naming **no** document is refused, and one naming a *trashed* document is refused with the path it
  had. A label is a word; a document id is minted by the system, so one that resolves nowhere means the
  caller is working from a stale read, and a reference a reader cannot open is worse than a refusal;
- a reference is **never cleaned up** when a document is deleted. Restoring is one call away, and a reference
  quietly dropped would not come back with the document. A reader that cannot resolve one is told it is in
  the trash.

Validating an added id is the one piece of validation in `scripts/` that reads Postgres — documents are not
in memory. It happens before a task number is reserved, like everything else, so a refused reference does not
burn an id.

In the browser: a **Documents** screen with the tree on the left and the selected document on the right,
modelled on the file browser in `remote-development-mcp` down to the class names — it is the same problem, and
a second design for it would be a second thing to maintain. The selection lives in the URL
(`/documents?selected=<id>`), so a document is linkable, survives a reload and works with the back button;
which folders are open is remembered per project in local storage, and every folder down to a linked document
is opened so a link lands ON it. How a document is drawn is decided by its **content type**, not by which column it came out of — html is text,
and a viewer that framed only binary payloads showed a web page as a wall of markup. Markdown is rendered with
a source toggle, an image is drawn in place, html and a PDF are framed with a way out to a full tab, any other
text is shown as it was written, and anything else is offered as a download.

**HTML is sandboxed and a PDF is not**, and the difference is deliberate. HTML served from our own origin
executes in it, and the session token lives in that origin's local storage — so a document an agent uploaded
could read every viewer's token. The frame carries `sandbox="allow-scripts"` and the raw response carries
`Content-Security-Policy: sandbox allow-scripts`: scripts and stylesheets still run, so the page renders as the
page it is, but it sits in an opaque origin. `allow-scripts` WITHOUT `allow-same-origin` is the combination
that matters — granting both lets the framed document drop the sandbox itself. The header as well as the
attribute, because "open full screen" loads that url directly in a tab, where the attribute does not exist. A
PDF gets none of it: its scripts run inside the browser's PDF viewer rather than in our page, and a sandbox
there breaks the viewer for no gain.

The file browser this viewer copies deliberately does NOT sandbox, and its comment names the condition for
changing that — a different exposure. This is one: a board on the public internet, behind a sign-in, showing
documents somebody else uploaded.

The raw bytes are served at **`/raw/{prefix}/{path}`** — the project in a path segment and the document's path
mirroring the tree, which is what makes a framed html page work: a page asks for its own `style.css` with a
relative url, and the browser resolves it against the address the page came from, so from
`/raw/TM/docs/page.html` it resolves to `/raw/TM/docs/style.css` and arrives. From a query url
(`?project=TM&id=…`) it would resolve back onto the api route and arrive as nothing. A path of arbitrary depth
is not something the routing macro can express, which is why this one route is a middleware rather than an
action — and it is the same reason the project is a segment here while every other endpoint takes it as a
parameter.

A reference on a task or a goal is a row labelled by *id* — the honest shape of what the client knows, since
the snapshot carries ids and not documents — and clicking it LEAVES the dialog for that screen rather than
opening a window over it: a PDF wants the pane the browser's viewer needs, which a modal over a board cannot
give it.

Nothing on that screen is live, deliberately: the socket carries the board, and documents must not ride along,
so Refresh is the answer to "an agent just uploaded something".

**Uploading is the second exception in the product to "MCP writes, the UI reads"** — the first being a goal's
colour. It earns it: the alternative is not a person using MCP, it is a person unable to upload at all, because
a PDF on a laptop cannot reach an agent without being base64-ed by hand into a tool call. Everything else about
a document — moving, deleting, restoring — is still MCP only, and the browser's upload is a second DOOR into
the same write path rather than a second path: uploading onto a taken path writes a new version of what is
there, keeping its id and its history, exactly as the tool does. The author comes from the session rather than
from an argument, which is the one thing this side does better than MCP.

The form has no "create folder", because there is nothing to create: a folder exists for as long as a document
is in it. Choosing an existing folder and making a new one are one act — typing a path — and the dropdown
beside the box is a shortcut that fills it in.

**The `(project_id, doc_path)` index is unique**, as the backstop under the path-is-the-key rule: the
application checks before it writes, and the index is what stops two writes racing past that check.

### A connected repository is a folder you can read and not write

Reference material usually already exists, and it usually already lives in a GitHub repository — a
specification, an API contract, somebody else's README that four tasks refer to. Copying it in by hand means
copying it in again every time it changes, which nobody does, which is how a project ends up with a
specification that is quietly a year old.

So a project can **connect** a repository. It takes a name, a url, optionally a branch and a folder inside the
repository, and from then on that repository appears in the project's documents as `github/<name>/…`,
refreshed every ten minutes.

**It is served by the tools that serve the project's own documents, and that is the whole point of the
design.** `documents_list` answers with the mirrored files beside the real ones, sorted into the same path
order; `documents_get` reads one by id or by path; `documents_outline` works on one. An agent needs to learn
nothing new — it asks the same question and gets the reference material with the rest.

**Everything that writes refuses them, by both names.** A mirrored file has no version and no history here,
so an upload, a move, a delete, a restore, an edit, a history or a diff naming one — by its `github/` path or
by its `github:`-prefixed id — is refused with a sentence saying where to go instead. There is one guard,
called from every write, rather than a rule each of them remembers.

**Bringing a file in for real is a sync, and a sync is a copy.** Pick files and folders out of the mirror,
choose a folder of the project's own, and what lands is a document with an id, a version, an author and a full
history — the same write an upload makes. It stops tracking the repository the moment it is written. The
`override` checkbox is the difference between the two ways this gets used: off writes only what is not already
there and reports the rest as skipped, which is safe to press repeatedly and never overwrites something
somebody has since edited; on writes a new version over every chosen path. Off is the default, because the
destructive reading of a button pressed by mistake should be the one you have to ask for.

**The configuration is durable and the key is not, deliberately.** The repository, branch and folder are a
`jsonb` column on the project row and survive everything. The token is held in the process's memory and is
written to no table, no settings file and no log — so a database dump carries no credential, and the cost is
that a private repository stops mirroring after a restart until somebody types the key again. A connection
reading `needs-key` on Monday having worked all Friday is that trade, not a bug. A public repository mirrors
anonymously and needs no key at all.

Everything downloaded lives in the container's temp directory, wiped at startup: whatever is in there belongs
to a process that is no longer running, and its connections may have been retargeted or deleted while nobody
was looking. Rebuilding costs one pull per connection and is the only way to be sure the disk agrees with the
configuration.

**The poll is cheap because it asks before it downloads.** Each tick reads the head commit — a few hundred
bytes — and only fetches the archive when the sha has moved. GitHub answers the archive route with a redirect
to codeload, which is followed by hand: FlUrl's native backend does not follow redirects at all, and the key
is deliberately dropped on the hop, because the redirect url is already signed and forwarding a credential to
another host is how tokens leak.

**A failed pull never empties a mirror that worked.** The entries and the commit are what the last successful
pull left; the state and the error describe the last attempt. A repository that goes unreachable overnight is
a warning beside a folder somebody can still read at nine in the morning.

What the mirror deliberately does not hold is counted rather than listed: a file over the single-document
size limit, a path this product will not name, anything past the five-thousand-file or 128 MiB ceilings. One
number answers the only question that matters about them — is the file I want missing because it was filtered,
or because it is not there?

Two things it is worth knowing it does **not** do. `documents_search` does not reach into mirrors — it
searches the project's own texts out of Postgres, and a mirrored file is on disk. And a key supplied for a
private repository fills a mirror that every member of the project can read, so the access it grants is
shared with them for as long as it is held.

## The session is a cookie

`HttpOnly`, `Secure`, `SameSite`, `Path=/`, and it expires with the token inside it. It replaced a token in
local storage carried in an `Authorization` header, and the move bought three things that header could not:

* **The browser attaches it to requests our code does not make.** An `<img>` or an `<iframe>` pointed at a
  document's bytes, and the WebSocket handshake. All three used to need the token spelled into their url —
  where it lands in browser history, in `Referer`, in proxy logs, and in any link somebody copied and sent on.
  A raw document url is now safe to hand to a colleague: it opens for them only if they are signed in and on
  that board.
* **Script cannot read it.** Which matters here specifically, because this product renders html somebody else
  uploaded. A token in local storage is a token an uploaded page can take.
* **`fetch` sends it with no help from us** — its default is `credentials: "same-origin"` and every api url is
  relative, so the client attaches nothing and therefore cannot leak anything.

The `Authorization` header is still accepted, and the WebSocket still reads a `token=` query. Both are there so
the deploy did not sign everybody out mid-session, and both can go once no live token predates the change.

**Which board is open is NOT a cookie**, and it was one for exactly one release. It is a preference of one
browser's, so it lives in that browser's local storage — **by prefix, not by id** — and the picker is
initialised from it on every load. Two things were wrong with the cookie. The server never read it, because it
had no reason to: every request that acts on a project names the project it acts on, in the body or in the
`/raw/{prefix}/{path}` url, and checks membership of that one. And it rode on every single request regardless,
including the ones fetching a document's bytes.

The bug it left behind is worth recording, because it is the shape of bug this whole boundary invites: what was
stored was the PREFIX, and the Home screen compared it against a project's internal ID. That comparison can only
fail, so every reload fell through to whichever board sorted first and the picker silently forgot what somebody
had chosen. The remembered prefix now goes through the same resolution a task handle does, which also means a
board renamed since it was remembered is still found, and one somebody lost access to falls through instead of
leaving the screen on nothing.

The other preference in local storage is which folders of a document tree are open — it can be dozens of paths,
and neither of the two is anything the server has a use for.

## A project is named by its prefix, and there is no id to get wrong

That bug had a root, and it was two vocabularies for one thing. The fix is that **the internal project id does
not cross the boundary at all** — not in a response, not in a request, not on the socket:

* `ProjectResponse` has **no `id` field**. `prefix` is the identity, and the picker, the api layer and the
  remembered board all hold that one string.
* Every input model names a project as `project` — `RMS` — and the action resolves it through
  `require_project_by_prefix`, which does the lookup and the membership check as one step so neither can be
  done without the other. Below that line everything still speaks in ids: the board, the scripts, Postgres.
* Every response names the board it came from the same way: `TaskResponse.project`, `GoalResponse.project`,
  `DocumentResponse.project`, `BoardSnapshot.project`. A task's own id already carried the prefix in front of
  its number, so the two now agree by construction.
* The socket's `{"watch":"RMS"}` is a prefix too. It is resolved once, at subscribe, and the subscription then
  holds the id — the one place the id is the better half of the trade, because the push comes from the write
  side, which knows the project by id, and a subscription pinned to it survives a prefix rename instead of
  going quiet until the tab is reloaded.
* `/raw/{prefix}/{path}` was already this shape, for a different reason — relative asset references inside a
  framed html document — and it is now the same shape as everything else rather than the exception.

**The cost, said out loud: a prefix is renameable, so this is identity that can change under a client.** A
rename invalidates whatever a browser is holding, and the screen recovers on its next read of the project list;
`prefix_history` is what keeps an old TASK id resolving regardless. That is the same deal every MCP tool has
always had, and it buys something worth more than immutability at this boundary: there is only one name for a
board, so there is nothing left to translate, and nothing left to translate backwards.

One consequence for deploys: the field names on the wire changed, so **the UI and the API go out together.** An
old UI against the new API sends `projectId` where `project` is expected and reads an `id` that is no longer
there.

## Deleting is a flag

A task and a goal each carry a `deleted_moment`: a moment rather than a bool, because "deleted" and "deleted
when" are one fact and two columns for it can disagree — the same reason `close_moment` is shaped that way.

**Nothing is removed, from Postgres or from memory, and that is the whole point.** What is deleted drops out of
every board, list, count and derived answer, and stays exactly where it was so that SEARCHING still finds it
and reports it as gone. A row deleted outright could only answer "no such task", which is indistinguishable
from a typo and from another board's id — and the moment anybody wants a deleted task is precisely the moment
they are searching for its id.

So the split is: `tasks_of_project` keeps deleted work, because it fills the snapshot the browser searches;
`get_task`, `get_goal`, `goals_of_project`, `goal_progress`, `tasks_of_goal`, `tasks_amount`,
`labels_of_project` and `blocks` all forget it. Search has its own doors — `get_task_including_deleted` and
`get_goal_including_deleted` — and the screens hide deleted work unless the search box has something in it, in
which case it is drawn faded and flagged.

Two consequences worth stating, because both fall out rather than being written:

* **A deleted blocker keeps its dependents blocked.** `is_blocked` treats an id naming no task as unsatisfied —
  deliberately, so a typo cannot silently free work — and a deletion lands in the same place. Deleting a
  blocker is not a statement that it was finished.
* **A deleted goal's tasks are not deleted with it.** They read as standalone, because `effective_goal` no
  longer resolves. Whether work outlives its container is a decision somebody makes explicitly, not a side
  effect of removing the container.

**Deleting a goal is not closing it.** Closing says how a goal WENT — which is why it demands a resolution and
refuses while any task is open. Deleting says it should never have existed, and asks nothing, because there is
nothing to record about work that was never real. A goal can be deleted whether it was open or closed.

Both undo cleanly: `deleted: false` on `tasks_update` or `goals_update`. Deleting twice does not move the
moment.

## Who is who

**Authentication is Google OAuth.** `client_id`, `client_secret` and `redirect_uri` come from the
service settings, not from the database — that is what keeps the service able to authenticate on
a cold start.

**A session is carried inside its token**, not stored anywhere: a protobuf model (`email`, `expires`)
encrypted with `session_encryption_key` from the settings, base64'd, and sent back as the bearer token.
The OAuth CSRF `state` is the same shape. So a restart signs nobody out, there is no session table and
no session map, and a second instance would accept tokens issued by the first.

The cost is that a live token cannot be revoked, which makes `logout` a client-side discard. Two things
make that acceptable: the TTL is 12 hours, and every request re-reads the user row — so disabling
somebody locks them out on their very next call, which is what revocation would actually be for. The
same check runs on the WebSocket handshake, so an open tab stops receiving updates too. Rotating the
key is the blunt instrument that invalidates everything at once.

**A user lives in Postgres:** email, name, `disabled`, and an `admin` flag. The board shows the
**name**; a task is assigned by **email**.

**Admin is either of two things:** the `admin` flag on the user row, **or** membership of the
admin list in the service settings. The settings list is additive, not an alternative — it is the
emergency door and the bootstrap: on an empty database nobody is an admin in Postgres, yet those
emails still get in and create everyone else. Same idea as `super_admins` in
`rms/dashboards-rest-api`.

**Access to a project is binary** — you are a member or you are not. Membership is edited from the
project's side, in Projects setup. An admin sees every project without being a member. The
**Users** screen is a roster: add, rename, disable. A disabled user cannot sign in; their
assignments and their comment authorship stay exactly where they are.

A user who is a member of nothing sees an empty Home and a note to ask an admin — not a 403 on
the whole UI.

**Only an admin edits a project** — prefix, columns, kinds, membership.

## MCP surface

Mounted on the same service-sdk HTTP server at `/mcp`, alongside the REST controllers.

**Everything is named by its human handle.** A project is `RMS`, a task is `RMS-42`, and `depends_on`
is `["RMS-7"]`. The internal `SortableId` never crosses this boundary — it is a database and UI
detail. Handles resolve against the state at the moment of the call, which is exactly right for a
tool call and exactly wrong for a handle quoted from last month's chat; that second case is what
`tasks_resolve_id` is for.

Tools:

- `projects_list` — the boards, their columns, kinds and labels. Called first: everything else
  names a project or a task by an id this returns.
- `users_list` — who exists, with email and name. This is how a spoken first name becomes the
  email that goes into `assignee`.
- `tasks_list` / `tasks_create` / `tasks_update` / `tasks_delete`
- `tasks_search` — the board and its threads, by what was written on them. The counterpart to
  `tasks_list`: that one answers "what is in this column", this one answers "where did we discuss
  this". **It is the only way to see inside a comment thread without already knowing which card to
  open** — every listing reports `comments_amount` and not one comment, deliberately, so without this
  the reasoning behind every decision on the board is reachable only by opening threads one call at a
  time. Returns a headline and the matching lines rather than whole tasks, sorted most-matched first.
- `tasks_add_comment` / `tasks_get_comments`
- `labels_list`
- `documents_list` / `documents_get` / `documents_history` — the index without the texts, one document
  with its text (by id, or by project and path, and optionally at an old `version`), and every version a
  document has had. Split that way on purpose: a document can be a whole specification, and an agent that
  pulled all of them in to find one would have spent the context it needed for the work.
- `documents_search` / `documents_outline` / `documents_get`'s `from_line` / `to_line` / `max_bytes` —
  the three halves of reading a large document without reading it. See
  [Documents](#documents).
- `documents_edit` — change parts of a text without sending the rest of it, atomically.
- `documents_diff` — a unified diff between two versions, returning neither.
- `documents_upload` / `documents_update_path` — write a version at a path, and move a document without
  touching its text. Two calls rather than one, so the history can tell the two apart.
- `documents_delete` / `documents_trash` / `documents_restore` — the trash, which exists nowhere else: the
  browser does not show it.
- `tasks_resolve_id` — the counterpart to composing ids on read. Given a human-written `RMS-42` it
  answers in two parts: the **direct** hit (the project holding `RMS` right now, and the task's
  current id), and the **archived** ones — every project that used to hold `RMS`, whether task 42
  exists there, and what that task is called *today*. Without this the cost of a reusable prefix
  would be unrecoverable: someone quoting an id from an old chat would land on a different task and
  never know. With it, the tool says "`RMS-42` is now this, and it used to mean that".

Every tool's description is written as a **use case** — when to call it and why — not as a list of
its fields. That is what made the original board's prompt work, and it is the part worth copying.

**`/mcp` has no authorization in the first version.** It sees every project and writes to any of
them; it is closed by the perimeter alone, like `development-tasks-mcp` before it. The asymmetry
this creates — reads gated by Google login and project membership, writes gated by nothing — is
recorded in `TODO.md` as the first thing to fix.

The original `development-tasks-mcp` prompt is **not** portable: it is built on a single board,
four hard-coded statuses, four hard-coded kinds, emails resolved against the rms console's `users`
table, the `.s`-suffix rule, the `dev_mine` badge and a `/system/development-tasks?task=` link.
The ideas port; the text does not.

## UI

Dioxus CSR — a static bundle. One bar of product areas across the top, work area under it at the full
width of the window. Across the top rather than down the left side because of Home: the board is a row of
columns and the only thing it wants is horizontal room, and a left menu costs that room on every screen
to be visible on the one screen where the vertical space was free anyway.

Everything below the router goes through one `Shell` component, which owns the single question every
screen needs answered first (is anybody signed in) so no view has to handle "not asked yet" and none can
forget to.

The stylesheet is **generated**: `build.rs` concatenates `css/*.css` into `public/assets/app.css` through
`ci_utils::css::CssCompiler`. Edit the numbered sources — an edit to `app.css` survives exactly until the
next build.

| Area | |
|---|---|
| **Home** (root URL) | The board. A project dropdown on top — only projects you may see; an admin sees all — and the choice is remembered in `localStorage`. Filters by task type and by assignee, plus a search box: free text narrows the board in place, while a task id (`RMS-42`) is looked up on the server and opens as a card, because the answer may be on another board or closed longer than seven days ago and therefore not drawn at all. **Read-only:** nothing is edited with a mouse, anywhere. |
| **Projects setup** | Every project as one row — prefix, name, description, task count, which column template it follows, its task types, how many members. Editing is by dialog: **Edit** for what a project *is* (name, description, prefix, and which column template), then **Task types** and **Members**. Admin only. |
| **Users** | The roster. Admin only. |
| **Settings** | A menu of areas on the left, the chosen one on the right, with the area in the route (`/settings/column-templates`) so each is linkable and Back works between them. **Column templates** is where a board's columns are configured. **Diagnostics** is read-only: which `client_id` was picked up, which `redirect_uri` is expected, how many admins the settings list holds — the first thing worth reading when a sign-in fails. |

The admin areas are hidden from a non-admin. Cosmetic on its own, since the server refuses them anyway;
the point is not showing somebody three screens that all answer 403.

### The dialog pattern

One shape, everywhere: **a dialog is handed a model, editing builds a new model beside it, Save lights up
only when the two differ, and pressing Save hands the new model out through an `EventHandler`.** The
handler — owned by the page or by `RenderDialog`, never by the dialog — makes the request and refreshes.

Nothing is sent until Save, and what is sent is the whole thing. Adding a column or a task type is a local
edit, so a half-finished set never reaches the server and is never visible to anybody else; Cancel costs
nothing because nothing was sent; and one snapshot request replaced the three (add, update, delete) that
used to need an order to be applied in.

Save being disabled on an untouched form is the point rather than an oversight: comparing models — not
tracking "was touched" — means typing a value and typing it back leaves the button off, because there is
genuinely nothing to save.

The plumbing: one `DialogState` enum, a `RenderDialog` router mounted once in the shell, `dialog_template`
supplying the frame, Cancel and close, and a `DialogFeedback` signal carrying how the last submit went —
without which a failed save would leave a dialog looking busy for ever, since it has no other way to learn
the request came back. `DialogState` is a context signal of its own rather than a field of `AppState`, which
the guide would have it be: Dioxus subscribes per signal, not per field, so putting it in `AppState` would
make opening a dialog re-run Home's board read.

### Columns and task types live in templates, not in a project

A **column template** is a named set of columns, and a **task-type template** a named set of task types.
Both are defined once under Settings and followed by any number of A project carries only the id of each template it follows. One with no column template has a board of just
Todo and Done; one with no task-type template has no types to choose from. Both are legitimate states, not
errors — otherwise creating a project would require creating two templates first.

The indirection earns itself twice. The projects on one board mostly share a workflow, so per-project
columns meant typing the same four columns into every project and watching them drift. And a template is a
thing you change once and have every project follow.

In memory, `ProjectModel.columns` and `.kinds` are a **cache**, not the source of truth:
`BoardInner::rebuild_indexes` recomputes both from the templates on every write, so they cannot drift, and
editing a template moves every project following it within the same swap. That is what keeps the
indirection to one function — `has_column`, `effective_status`, `has_kind`, the board read and the MCP tools
all still ask a project for its own columns and types and never learn templates exist.

Deleting a template with any followers is refused. For columns the reason is sharp: a project whose template
vanished loses the middle of its board and every task sitting there reads as Todo. Losing a task type is
milder — a task pointing at one that is gone reads as having no type — but it still changes every following
project at once, so both are made deliberately rather than discovered.

### Everything is a POST with a body — no path parameters, no query values

`#[http_path]` fields are **appended** to the url in declaration order — `append_path_segment`, no `{name}`
substitution. So `/api/projects/v1/{project}/columns` is unreachable from the generated client: it can
only build `/api/projects/v1/{project}`, and the server answers 404. Seven of the ten project endpoints
were written that way and every one of them was dead on arrival.

So there is no `#[http_path]` anywhere in this repo. And no `#[http_query]` either, because a query value
has its own way of being mangled: **`+` in a query means a space.** Both halves of the OAuth callback are
base64-ish and regularly contain one, so sending them as query parameters silently corrupted the CSRF state
on roughly six sign-ins in ten — the ones whose state happened to contain a `+` — and surfaced as a 401 that
looked random and cleared itself on a retry.

Every endpoint the client calls is therefore a `POST` to a static url with everything in a JSON body, which
carries bytes verbatim. Reads too: `/api/projects/v1/list`, `/api/tasks/v1/list`. The lists live at `/list`
rather than at the collection root because create already owns the root as a POST, and two POST handlers on
one route is a collision.

One exception: `GET /api/system/v1/ping`. It is an infrastructure liveness probe reached by is-alive and the
proxy, not by the client — making it a POST would silently break the health check.

Sign-in is one button: `/api/auth/v1/google-url` → Google → back to `/authorized`, which exchanges the
code, stores the token and replaces the URL so a reload cannot re-submit a spent code. Signing out
navigates rather than routes, because it has to throw away the WebSocket task and the cached `/me` that a
route push would leave running under the login screen.

Home holds a WebSocket and repaints on a push. What travels is an **invalidation signal** — "this
project changed" — not a delta: the client re-reads through the normal REST call. Nothing can
drift out of sync, because there is no second copy of the state on the client. This matters more
than it would in a normal admin panel: every change originates outside the UI, so a static screen
would simply be lying.

Sticker text renders as Markdown (the `markdown` crate, which escapes raw HTML rather than passing
it through). A sticker links to itself as `?search=RMS-42` — the one button on a card copies that link, and
the board follows a handle found in the URL at mount to the task's own project and opens it. The link is the
search box written down, which is why there is no second parameter to keep working; the prefix is globally
unique, so the project is not part of it. An assignee email with no matching user row is shown as the raw
email.

## Running it

Runs on **HETZNER**, product namespace `task-manager-mcp`, host ports from the **31500+** range:
`31500 → 8000` for the REST API and `31501 → 9001` for the UI. Those container ports are not arbitrary —
service-sdk's HTTP server listens on 8000 (8888 is its second, technical port) and `web-app-host` serves
static files on 9001.

`release/settings-template.yaml` is the settings-service template
(`product_id = task-manager-mcp`, `template_id = task-manager-rest-api`) and
`release/secrets.md` is the inventory of every secret it references. Two entries will refuse to start the
service if they are wrong, on purpose:

- `SessionEncryptionKey` must be **exactly 48 bytes** — `AesKey` panics on any other length, and checking
  it at startup beats dying on the first sign-in hours later (`openssl rand -hex 24`);
- `Admins` must name at least one address, or nobody can get into an empty database.

`release/docker-compose.yaml` follows the standard single-VM unix-socket layout, with one deliberate
deviation from the template: the REST API's memory limit is 256Mb rather than the usual 64Mb, because it
holds the whole product in memory and replaces a snapshot on every write.

`task-manager-ui` calls the REST API on relative `/api/...` URLs and opens `/ws` on the same origin, so the
reverse proxy has to put both services behind **one** host — the UI never knows a base URL and there is
nothing in it to configure.

Once it is up, register the MCP surface with the client that will work the board:

```json
"task-manager": { "type": "http", "url": "https://<host>/mcp" }
```

Locally: `dx serve` in `task-manager-ui`, and `cargo run` in `task-manager-rest-api`.

### Releasing — the pre-baked builder image

`task-manager-rest-api` releases in about **two minutes** rather than ten, and the trick is that it never
rebuilds a dependency graph that has not changed. `build-task-manager-rest-api-docker.yaml` is
`workflow_dispatch`-only: it compiles service-sdk, my-postgres, my-http-server, tokio and jemalloc once
into `ghcr.io/my-ai-utils/task-manager-rest-api-build-docker:latest`, and every release mounts its fresh
checkout over that image and compiles only the delta.

**Run the builder workflow by hand whenever the GRAPH moves** — a MyJetTools tag bumped, a crate added,
`Cargo.lock` updated. Nothing else needs it. It is `workflow_dispatch` because there is no Docker daemon on
a dev machine, so a builder image can only be tested where it is built; iterating on it through release
tags would burn a version number per attempt.

Four things are load-bearing, and each one silently turns a warm build back into a cold one — no error,
just the old ten minutes:

- **`CARGO_HOME` and `CARGO_TARGET_DIR` live outside `/src`.** The release bind-mounts its checkout over
  `/src`, and a bind mount *hides* whatever the image had underneath. A target dir in there would vanish at
  exactly the moment it is meant to be reused;
- **the image bakes at `/src`, the same absolute path the release mounts.** Cargo fingerprints record
  absolute paths, so the same sources at another path are a cold build wearing a warm image's name;
- **the builder's base matches the runtime image's** (`ubuntu:22.04`). The binary is copied into the
  runtime image rather than rebuilt there, so a newer base links it against a newer glibc and the container
  dies at start-up on a symbol lookup;
- **`task-manager-rest-api/Cargo.lock` is committed**, un-ignored explicitly in `.gitignore`. With the lock
  ignored CI resolves fresh and takes the newest semver-compatible release of every transitive crate, so
  one patch published anywhere in a ~400-crate graph invalidates the image and warm hits become a lottery
  nobody can measure. It also means a broken transitive dependency reproduces locally instead of only in
  CI. Verify with `cargo check --locked` before committing a change to it.

The context is the **repo root**, not the service folder, because `task-manager-shared` is a path
dependency. Only those two crates recompile per release: `actions/checkout` stamps every file with the
checkout time, so cargo considers all of our own sources dirty regardless — which is fine, they are small.

**A release never breaks because of the builder.** The pull step is `continue-on-error`, and the cold steps
behind the `if:` are the exact build used before the image existed, token and all. The worst outcome of a
missing or broken image is the time we used to pay anyway. Confirm in a run that *Build (warm, …)* ran and
the cold steps were skipped.

Not for `task-manager-ui`: it is a Dioxus WASM build with its own toolchain and `dx build`, running inside
`ghcr.io/myjettools/dioxus-docker`. Different problem, different fix — and its lock stays ignored.

## Open

- The **share link** is built against the page's own origin, so it assumes the board is at the root of the
  public host — which it is: `https://task-manager.jetdev.eu/?search=RMS-42`.
- **Copying it needs a secure context.** `navigator.clipboard` exists on https and on localhost, which is
  both of the ways the board is reached; served over plain http the button shows the link in a dialog to be
  copied by hand instead.
- **No WebSocket reconnect.** A dropped socket leaves Home static until the page is reloaded. The dot in
  the header goes grey so it is visible rather than silent, but a laptop waking from sleep currently needs
  a refresh.
