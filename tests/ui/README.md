# UI smoke test

Build the actual Dioxus client in release mode so the development hot-reload socket is absent:

```sh
cd task-manager-ui
dx build --web --release
```

From `tests/ui`, install the pinned test dependencies with `npm ci`. Set `TASKS_UI_DIST` to the
`public` directory reported by the build. Use an installed browser with `TASKS_CHROME_PATH`, or
install Playwright's Chromium with `npx playwright install chromium`, then run `npm test`.

The script creates a temporary loopback HTTP/WebSocket server, uses only synthetic records and a
fresh headless browser profile, and stops both on completion. It does not call the deployed board
or an AI provider. Screenshots and the result report are written to the ignored `results` folder.

The scenarios cover opening a Goal by URL, actual dependency reasons, keeping task text readable,
answering through Inbox, preserving a saved answer across a failed refresh, form-label isolation,
narrow layouts and reconnecting with fresh task data and project settings. The server also counts
subscriptions so an update/subscription feedback loop fails the test.

Run `npm run test:settings` against the same built WASM for the AI/knowledge settings flow. It covers
provider save/test, keeping and removing stored credentials, disabled testing for unsaved edits,
project source configuration, Graphify raw upload, context preview, index progress/cancellation,
project isolation, administrator access and narrow layouts. All providers, keys and sources in this
browser suite are synthetic; the suite never calls a real embeddings or Jev provider.
