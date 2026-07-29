# task-manager-mcp

A task board that **agents work and humans configure**. Every task mutation arrives through MCP;
the browser UI only sets things up and watches.

That split is the whole design. It is not a Jira with an MCP bolted on — the MCP surface is the
primary interface, and the UI exists because projects, columns, kinds and people have to be
configured by a person, and because someone wants to see the board.

Product namespace on the host: `task-manager-mcp`. Successor to `rms/development-tasks-mcp`, which
this replaces once the first version lands.

## Two services

| | |
|---|---|
| **`task-manager-rest-api`** | service-sdk HTTP. Three surfaces on one port: `/api/v1/*` (reads for the UI + configuration CRUD), `/mcp` (every task mutation), `/ws` (invalidation push). Owns Postgres. |
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
| `prefix` | The human-facing half of a task id: `RMS` → `RMS-000042`. Renameable. |
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
`number`, and `RMS-000042` is composed on read from the project's *current* prefix. Nothing can
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

### Kinds

Same shape as columns — id entered by hand, name, description — plus a **colour picked from a preset
set**: an enum in `task-manager-shared`, validated by the server and drawn by the UI as clickable
swatches rather than a dropdown of colour names, because a colour picker that shows the colours beats one
that spells them. Unlike columns, kinds have no mandatory anchors: a project may have none.

### Labels

Per project, and nothing more than a name — no description, no rules, no meanings. A task picks
an existing one or introduces a new one by typing it. Consequently there is no label table: a
project's label set is the distinct labels in use across its tasks, and a label stops existing
when the last task drops it.

This is a deliberate simplification of `development-tasks-mcp`, where labels were a shared
vocabulary with written definitions, a `used_by` count and an "in use but undefined" report. That
machinery is gone.

## Task

| Field | |
|---|---|
| `project_id` | Owner. |
| `number` | Sequential within the project, from the project's own counter. Not reused after a delete. |
| — | The human id `PREFIX-000042` is **composed on read**, never stored — see above. Zero-padded; `RMS-1` typed by a human parses the same way. |
| `text` | Markdown — the UI renders it. |
| `status` | A column id of this project. Unknown → reads as `todo`. **Always `todo` on creation** — `tasks_create` takes no status. |
| `kind` | A kind id of this project. Optional. |
| `assignee` | An email, or `claude`. |
| `labels` | Free tags, lowercased and de-duplicated. |
| `depends_on` | **Numbers** of blocking tasks, within the same project. |
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
- `tasks_add_comment` / `tasks_get_comments`
- `labels_list`
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
| **Home** (root URL) | The board. A project dropdown on top — only projects you may see; an admin sees all — and the choice is remembered in `localStorage`. **Read-only:** nothing is edited with a mouse, anywhere. |
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

### No path parameters at all — every mutation is a POST with a body

`#[http_path]` fields are **appended** to the url in declaration order — `append_path_segment`, no `{name}`
substitution. So `/api/projects/v1/{projectId}/columns` is unreachable from the generated client: it can
only build `/api/projects/v1/{projectId}`, and the server answers 404. Seven of the ten project endpoints
were written that way and every one of them was dead on arrival.

So there is no `#[http_path]` anywhere in this repo. Every mutation is a `POST` to a **static** url with
everything in the body — `/api/column-templates/v1/save`, `/api/projects/v1/kinds/set` — which is also the
shape every other house service uses (`/promo-codes/save`, `/individual-offers/assign`). Reads stay `GET`
with query parameters, which have no such problem.

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
it through). A sticker links to itself as `?task=RMS-42`; the prefix is globally unique, so the
project is not part of the link. An assignee email with no matching user row is shown as the raw
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

## Open

- The **share link** shape assumes the board is at the root of the public host, which it is:
  `https://task-manager.jetdev.eu/?task=RMS-42`.
- **No WebSocket reconnect.** A dropped socket leaves Home static until the page is reloaded. The dot in
  the header goes grey so it is visible rather than silent, but a laptop waking from sleep currently needs
  a refresh.
- The **share link** (`?task=RMS-42`) is described here and not implemented in the UI yet.
