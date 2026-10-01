import assert from 'node:assert/strict';
import http from 'node:http';
import path from 'node:path';
import fs from 'node:fs';
import { chromium } from 'playwright';
import { WebSocketServer } from 'ws';

// Exercises the real compiled WASM with synthetic HTTP/WS fixtures only.
const dist = path.resolve(process.env.TASKS_UI_DIST || '../../task-manager-ui/target/dx/task-manager-ui/debug/web/public');
assert.ok(fs.existsSync(path.join(dist, 'index.html')), 'Build the UI with dx build and set TASKS_UI_DIST to its public directory');
const results = path.resolve('results');
fs.mkdirSync(results, { recursive: true });
const now = Math.floor(Date.now() / 1000);
let revision = now * 1_000_000;
const project = { name: 'Tasks development', description: 'Workflow clarity and decision history', prefix: 'DEMO', prefix_history: [],
  columns: [{ id: 'in-progress', name: 'In progress', description: '', order: 1 }], column_template_id: null, column_template_name: null,
  kinds: [{ id: 'task', name: 'Task', description: '', color: 'blue', icon: 'task' }], kind_template_id: null, kind_template_name: null,
  members: ['owner@example.test'], tasks_amount: 2, archive_days: 7, archived: false };
const question = (id, required = true) => ({ id, request_key: id, action: 'Publish the reviewed prototype', question: 'Which delivery scope should be used?',
  options: [{ id: 'preview', label: 'Local preview', consequence: 'Verify the prototype with synthetic data', recommended: true },
    { id: 'later', label: 'Wait for fixes', consequence: 'Keep the current task open', recommended: false }],
  required, asked_by: 'Codex', asked_unix_seconds: now - 3600, status: 'pending', answer: null, cancel_reason: null, cancelled_by: null, cancelled_unix_seconds: null });
const task = (number, text) => ({ id: `DEMO-${number}`, project: 'DEMO', text, status: 'todo', priority: 'normal', kind: 'task', goal: 'DEMO-G3',
  goal_name: 'Deliver a reviewable workflow', goal_color: 'blue', assignee: 'owner@example.test', assignee_name: 'Owner', labels: [],
  depends_on: [], blocks: [], link_statuses: [], blocked: false, documents: [], subtasks: [], gh_actions: [], comments: [],
  created_unix_seconds: now - 7200, updated_unix_seconds: now, revision_unix_microseconds: revision, closed_unix_seconds: null,
  deleted_unix_seconds: null, execution_prompts: [], decisions: [], analysis_documents: [], ai_reviews: [],
  readiness: { dependencies: [], required_decisions: 0, analysis_required: false } });
const first = task(1, 'Ship workflow improvements\n\nReview the dependency, confirm the delivery scope and attach the analysis results.');
const blocker = task(2, 'Finish acceptance checks');
first.depends_on = ['DEMO-2']; first.decisions = [question('q1')];
first.ai_reviews = [{ id: 'a'.repeat(64), input_hash: 'a'.repeat(64), requested_model: 'jev-latest', model: 'jev-1.13.0', policy_version: 'task-triage-v1',
  who: 'owner@example.test', created_unix_seconds: now - 90, source: 'jev', suggested_kind: 'task', kind_probability: 0.9, kind_confidence: 0.8,
  suggested_priority: 'normal', urgency_score: 2, urgency_confidence: 0.8, needs_human_probability: 0.9,
  suggested_duplicate: null, duplicate_probability: 1, duplicate_confidence: 1, review_required: true }];
const tasks = [first, blocker];
function refreshFacts() {
  first.blocked = blocker.status !== 'done';
  first.link_statuses = [{ id: blocker.id, status: blocker.status }];
  first.readiness = { dependencies: first.blocked ? [{ task_id: blocker.id, title: blocker.text, status: blocker.status, assignee: blocker.assignee }] : [],
    required_decisions: first.decisions.filter(d => d.required && d.status === 'pending').length, analysis_required: true };
}
refreshFacts();
function goal() {
  return { id: 'DEMO-G3', project: 'DEMO', name: 'Deliver a reviewable workflow', description: 'A synthetic goal used by the browser smoke test.',
    color: 'blue', priority: 'normal', status: 'in-progress', tasks_amount: 2, done_amount: tasks.filter(t => t.status === 'done').length,
    documents: [], subtasks: [], comments: [], created_unix_seconds: now - 7200, updated_unix_seconds: now,
    revision_unix_microseconds: revision,
    closed_unix_seconds: null, deleted_unix_seconds: null,
    waiting_tasks: [{ task_id: first.id, title: first.text.split('\n')[0], readiness: first.readiness }] };
}
function snapshot() { refreshFacts(); return { boardSnapshot: { project: 'DEMO', tasks, goals: [goal()] }, projectChanged: 'DEMO' }; }
const wss = new WebSocketServer({ noServer: true });
let socketsOpened = 0, answersPosted = 0, watchesReceived = 0, unavailableUntil = 0, failNextFind = false;
function push() { const payload = JSON.stringify(snapshot()); for (const socket of wss.clients) if (socket.readyState === 1) socket.send(payload); }
wss.on('connection', socket => {
  socketsOpened++;
  socket.on('message', raw => {
    const message = JSON.parse(raw.toString());
    if (message.watch) { watchesReceived++; socket.send(JSON.stringify(snapshot())); }
    if (message.ping) socket.send('{"pong":true}');
  });
});
const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://localhost');
    const send = (status, body) => { res.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' }); res.end(JSON.stringify(body)); };
    if (url.pathname.startsWith('/api/')) {
      let raw = ''; for await (const chunk of req) raw += chunk;
      const body = raw ? JSON.parse(raw) : {};
      refreshFacts();
      switch (url.pathname) {
        case '/api/auth/v1/me': return send(200, { email: 'owner@example.test', name: 'Owner', is_admin: true });
        case '/api/projects/v1/list': return send(200, { projects: [project] });
        case '/api/tasks/v1/list': return send(200, { tasks });
        case '/api/goals/v1/list': return send(200, { goals: [goal()] });
        case '/api/tasks/v1/decisions/pending': return send(200, { items: first.decisions.filter(d => d.status === 'pending').map(decision =>
          ({ task_id: first.id, task_title: first.text.split('\n')[0], project: 'DEMO', project_name: project.name, decision })) });
        case '/api/tasks/v1/decision/answer': {
          answersPosted++;
          const decision = first.decisions.find(d => d.id === body.decisionId);
          assert.ok(decision, 'answer names a recorded question');
          assert.equal(decision.status, 'pending', 'no duplicate answer was submitted');
          decision.status = 'answered';
          decision.answer = { option_id: body.optionId || null, text: body.text, answered_by: 'owner@example.test', answered_unix_seconds: now, source: 'ui' };
          first.revision_unix_microseconds = ++revision;
          push();
          res.writeHead(202); return res.end();
        }
        case '/api/tasks/v1/find': {
          if (failNextFind) { failNextFind = false; return send(503, { message: 'Synthetic refresh failure after save' }); }
          return send(200, { task: tasks.find(t => t.id === body.query) || null, goal: body.query === 'DEMO-G3' ? goal() : null,
            project: 'DEMO', project_name: project.name, archived: false, not_found: '' });
        }
        default: return send(404, { error: `Unmocked API path: ${url.pathname}` });
      }
    }
    let file = path.resolve(dist, `.${decodeURIComponent(url.pathname)}`);
    if (file !== dist && !file.startsWith(dist + path.sep)) { res.writeHead(403); return res.end(); }
    if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) file = path.join(dist, 'index.html');
    const mime = { '.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png' }[path.extname(file)] || 'application/octet-stream';
    res.writeHead(200, { 'Content-Type': mime, 'Cache-Control': 'no-store' }); fs.createReadStream(file).pipe(res);
  } catch (error) { res.writeHead(500); res.end(String(error)); }
});
server.on('upgrade', (req, socket, head) => {
  if (req.url !== '/ws') return socket.destroy();
  if (Date.now() < unavailableUntil) return socket.destroy();
  wss.handleUpgrade(req, socket, head, ws => wss.emit('connection', ws, req));
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const base = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true, ...(process.env.TASKS_CHROME_PATH ? { executablePath: process.env.TASKS_CHROME_PATH } : {}) });
const page = await browser.newPage({ viewport: { width: 1440, height: 1050 }, timezoneId: 'Asia/Dubai' });
const errors = [];
let stage = 'initial';
page.on('pageerror', error => errors.push({ stage, message: String(error), stack: error.stack }));
await page.route('**/*', route => route.request().url().startsWith(base) ? route.continue() : route.abort());
try {
  await page.goto(`${base}/?search=DEMO-G3`);
  await page.getByText('Tasks needing attention (1)', { exact: true }).waitFor();
  await page.locator('.goal-waiting-task summary').click();
  await page.getByRole('heading', { name: 'Blocked by', exact: true }).waitFor();
  await page.screenshot({ path: path.join(results, 'goal-blockers.png'), fullPage: true });

  stage = 'task';
  await page.goto(`${base}/?search=DEMO-1`);
  await page.getByRole('heading', { name: 'Blocked by', exact: true }).waitFor();
  await page.getByText('Current status: todo. Complete this dependency to unblock the task.').waitFor();
  const taskBody = page.locator('.task-view-text');
  assert.match(await taskBody.innerText(), /Review the dependency/);
  assert.ok((await taskBody.boundingBox()).height > 10, 'readiness must not collapse the task specification');
  await page.screenshot({ path: path.join(results, 'task-readiness.png'), fullPage: true });
  await page.getByTitle('Close', { exact: true }).click();
  stage = 'inbox';
  await page.getByRole('link', { name: 'Inbox', exact: true }).click();
  await page.getByRole('button', { name: 'Record answer', exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: 'Record answer', exact: true }).isDisabled(), true, 'recommendations are never submitted by default');
  await page.screenshot({ path: path.join(results, 'inbox.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1), 'inbox fits a narrow viewport');
  await page.screenshot({ path: path.join(results, 'inbox-mobile.png'), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1050 });
  await page.getByLabel('Your choice', { exact: true }).selectOption('preview');
  await page.getByLabel('Answer or note', { exact: true }).fill('Use the local prototype.');
  await page.getByRole('button', { name: 'Record answer', exact: true }).click();
  await page.getByText('No questions waiting for you.', { exact: true }).waitFor();
  assert.equal(answersPosted, 1);
  assert.equal(first.decisions[0].answer.text, 'Use the local prototype.');

  first.decisions.push(question('q2', false));
  stage = 'answer refresh failure';
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.getByRole('button', { name: 'DEMO-1 · Ship workflow improvements', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Your choice', { exact: true }).selectOption('later');
  failNextFind = true;
  await dialog.getByRole('button', { name: 'Record answer', exact: true }).click();
  await dialog.getByText('Answer recorded.', { exact: true }).waitFor();
  assert.equal(await dialog.getByRole('button', { name: 'Record answer', exact: true }).count(), 0, 'a refresh failure cannot resubmit a saved answer');
  assert.equal(answersPosted, 2);

  stage = 'reconnect';
  await page.goto(base);
  await page.getByText('Live', { exact: true }).waitFor();
  assert.ok(watchesReceived <= socketsOpened * 3, 'identical snapshots must not trigger a subscribe/snapshot feedback loop');
  const openedBefore = socketsOpened;
  unavailableUntil = Date.now() + 1200;
  for (const socket of wss.clients) socket.terminate();
  blocker.status = 'done'; blocker.closed_unix_seconds = now;
  project.description = 'Project settings refreshed after reconnect';
  first.text = 'Recovered snapshot after tunnel reconnect'; first.revision_unix_microseconds = ++revision;
  await page.getByText('Recovered snapshot after tunnel reconnect', { exact: true }).waitFor({ timeout: 20000 });
  assert.ok(socketsOpened > openedBefore, 'the board reconnected');
  await page.getByText('Live', { exact: true }).waitFor();
  await page.getByText('Project settings refreshed after reconnect', { exact: true }).waitFor();
  await page.screenshot({ path: path.join(results, 'reconnected-board.png'), fullPage: true });
  assert.deepEqual(errors, []);
  const report = { passed: ['goal blockers', 'task readiness', 'inbox answer and immutable history', 'no automatic answer', 'save/refresh failure recovery', 'reconnect with fresh snapshot', 'no subscription feedback loop', 'narrow inbox layout'], answersPosted, socketsOpened, watchesReceived, pageErrors: errors };
  fs.writeFileSync(path.join(results, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  await page.screenshot({ path: path.join(results, 'failure.png'), fullPage: true }).catch(() => {});
  fs.writeFileSync(path.join(results, 'failure.txt'), `${error}\n${JSON.stringify(errors, null, 2)}\n${await page.locator('body').innerText().catch(() => '')}`);
  throw error;
} finally {
  await browser.close();
  for (const socket of wss.clients) socket.terminate();
  await new Promise(resolve => wss.close(resolve));
  await new Promise(resolve => server.close(resolve));
}
