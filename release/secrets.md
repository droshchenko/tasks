# Secrets for `task-manager-mcp`

Created in settings-service, product scope `task-manager-mcp`. **Nothing here holds a real value and
nothing here should ever hold one** — values live in settings-service; this file is the map, not the safe.

Ids are kebab-case with no product prefix, matching the convention in `trading`: the product scope already
namespaces them, so `logger`, not `task-manager-logger`.

| Id | Value | Readable by MCP | What it is |
|---|---|---|---|
| `logger` | **set** | yes | MyLogger sink: `url=http://my-logger:8000;flushlogschunk=50;flushdelay=3`. Taken from the `mt-risks` template — same host, same `my-logger` container over `docker_net`. Read as `seq_conn_string`. |
| `postgres-conn-string` | `fill it` | no | Needs its **own** database and user: the service creates and verifies its own tables at startup, so it must own the schema. HETZNER runs `postgres:18.0` as container `db` → `host=db port=5432`. Interpolate the password from a separate secret rather than inlining it, the way `trading/postgres-conn-string` does. |
| `google-client-id` | `fill it` | yes | Google console → Credentials → OAuth 2.0 Client ID, type **Web application**. Readable on purpose: the Settings screen returns it in full, because a truncated client id cannot be compared against the console. |
| `google-client-secret` | `fill it` | no | The secret half of the same client. Never returned by any endpoint. |
| `google-redirect-uri` | **set** | yes | `https://task-manager.jetdev.eu/authorized`. Must match an authorised redirect URI in the console **byte for byte** — and the UI's `/authorized` route has to match it too, which is why the route is not free to rename. |
| `session-encryption-key` | `fill it` | no | **Exactly 48 bytes** — `openssl rand -hex 24`. The placeholder is 7 bytes and **will fail startup**, which is deliberate: the service checks the length at boot rather than dying on the first sign-in. |

## Not a secret

`admins` is a literal list in the template: a list rather than a scalar, and an admin email is not a
credential. Still load-bearing — empty list **and** empty roster locks everybody out of a fresh deployment.

`my_telemetry` is omitted entirely. It is `Option<String>` on the model, a missing field reads as `None`,
and every other template on these hosts omits it too.

## Litter to delete

Five secrets were created under the wrong naming convention before the `trading` product was inspected.
They are unused — the template references none of them — but settings-service exposes no delete, so they
need removing through the UI:

`task_manager_postgres`, `task_manager_google_client_id`, `task_manager_google_client_secret`,
`task_manager_google_redirect_uri`, `task_manager_session_key`
