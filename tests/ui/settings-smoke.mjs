import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { chromium } from 'playwright';

const dist = path.resolve(process.env.TASKS_UI_DIST || '../../task-manager-ui/target/dx/task-manager-ui/release/web/public');
assert.ok(fs.existsSync(path.join(dist, 'index.html')), 'Build the release WASM first');
const results = path.resolve('results/settings'); fs.mkdirSync(results, { recursive: true });
const now = Math.floor(Date.now() / 1000), commit = 'a'.repeat(40);
const secret = 'SYNTHETIC_SETTINGS_TEST_CREDENTIAL';
let admin = true, saves = 0, probes = 0, graphUploads = 0, indexStarts = 0, cancellations = 0;
const calls = [], errors = [], keys = new Map();
const provider = name => ({ provider: name, enabled: false, endpoint: name === 'jev' ? 'https://api.typesafe.ai/v1/systemone' : '',
  model: name === 'jev' ? 'jev-latest' : '', has_key: false, source: 'environment', revision: 0, configured: false, notice: 'Provider is disabled.' });
const settings = { embeddings: provider('embeddings'), jev: provider('jev') };
const project = prefix => ({ name: `${prefix} project`, description: '', prefix, prefix_history: [], columns: [], column_template_id: null, column_template_name: null,
  kinds: [], kind_template_id: null, kind_template_name: null, members: ['owner@example.test'], tasks_amount: 3, archive_days: 7, archived: false });
const projects = [project('DEMO'), project('OTHER')];
const configs = Object.fromEntries(projects.map(p => [p.prefix, { project: p.prefix, revision: 0,
  config: { enabled: false, connection: '', wiki_paths: [], graph_source: 'none', graph_path: '', auto_refresh: true },
  connections: [{ name: 'source', repository: `example/${p.prefix.toLowerCase()}`, branch: 'main', root_path: '', state: 'ready', commit: commit.slice(0, 7) }],
  status: { state: 'disabled', wiki_documents: 0, graph_nodes: 0, graph_edges: 0, source_commit: '', graph_commit: '', updated_unix_seconds: null, notice: '' } }]));
const jobs = Object.fromEntries(projects.map(p => [p.prefix, { state: 'idle', processed: 0, total: 0, remaining: 0, started_unix_seconds: null, finished_unix_seconds: null, message: '' }]));
const vectors = { DEMO: 0, OTHER: 0 };
let slowIndex = false;
function indexStatus(prefix) { return { project: prefix, configured: settings.embeddings.configured, tasks_total: 3, vectors_current: vectors[prefix],
  vectors_missing: 3 - vectors[prefix], model: settings.embeddings.model, notice: '', job: jobs[prefix] }; }
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
  const send = (status, value) => {
    const body = JSON.stringify(value);
    assert.ok(!body.includes(secret), 'credentials never appear in API responses');
    res.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' }); res.end(body);
  };
  try {
    if (url.pathname.startsWith('/api/')) {
      const chunks = []; for await (const chunk of req) chunks.push(chunk);
      const raw = Buffer.concat(chunks);
      if (url.pathname === '/api/auth/v1/me') return send(200, { email: 'owner@example.test', name: 'Owner', is_admin: admin });
      if (url.pathname === '/api/projects/v1/list') return send(200, { projects });
      if (!admin) return send(403, { message: 'Admins only.' });
      if (url.pathname === '/api/settings/knowledge/v1/graph') {
        const prefix = url.searchParams.get('project'), config = configs[prefix];
        assert.ok(config); assert.equal(Number(url.searchParams.get('revision')), config.revision);
        const graph = JSON.parse(raw.toString()); graphUploads++;
        assert.equal(graph.built_at_commit, commit);
        config.status.graph_nodes = graph.nodes.length; config.status.graph_edges = graph.links.length;
        config.status.graph_commit = graph.built_at_commit; config.status.state = 'ready'; config.status.notice = '';
        return send(200, config);
      }
      const body = raw.length ? JSON.parse(raw.toString()) : {};
      calls.push({ path: url.pathname, project: body.project });
      switch (url.pathname) {
        case '/api/settings/ai/v1/get': return send(200, settings);
        case '/api/settings/ai/v1/save': {
          const current = settings[body.provider]; assert.ok(current); assert.equal(body.revision, current.revision);
          if (body.apiKey) keys.set(body.provider, body.apiKey);
          if (body.clearKey) keys.delete(body.provider);
          Object.assign(current, { enabled: body.enabled, endpoint: body.endpoint, model: body.model, has_key: keys.has(body.provider), source: 'settings', revision: current.revision + 1 });
          current.configured = current.enabled && Boolean(current.endpoint && current.model) && (body.provider !== 'jev' || current.has_key);
          current.notice = current.configured ? '' : 'Provider is disabled.'; saves++;
          return send(200, settings);
        }
        case '/api/settings/ai/v1/test': probes++; return send(200, { success: true, model: settings[body.provider].model, message: 'Synthetic provider connection verified.', elapsed_ms: 12 });
        case '/api/settings/knowledge/v1/get': return send(200, configs[body.project]);
        case '/api/settings/knowledge/v1/save': {
          const current = configs[body.project]; assert.equal(body.revision, current.revision);
          current.revision++; current.config = { enabled: body.enabled, connection: body.connection, wiki_paths: body.wikiPaths,
            graph_source: body.graphSource, graph_path: body.graphPath, auto_refresh: body.autoRefresh };
          current.status.state = body.enabled ? 'not-indexed' : 'disabled'; return send(200, current);
        }
        case '/api/settings/knowledge/v1/refresh': {
          const current = configs[body.project];
          Object.assign(current.status, { state: 'partial', wiki_documents: 2, source_commit: commit, updated_unix_seconds: now, notice: 'Upload a current Graphify export.' });
          return send(200, current);
        }
        case '/api/settings/knowledge/v1/preview': return send(200, { hits: [{ source: 'wiki', reference: `raw/${body.project}/github/source/docs/wiki/queues.md`,
          title: 'Queue architecture', excerpt: 'Valkey holds the project job queues.', content_hash: 'b'.repeat(64), commit }], notice: '' });
        case '/api/settings/ai/v1/index/status': return send(200, indexStatus(body.project));
        case '/api/settings/ai/v1/index/start': {
          indexStarts++; jobs[body.project] = { state: 'running', processed: 0, total: 3, remaining: 3, started_unix_seconds: now, finished_unix_seconds: null, message: '' };
          if (!slowIndex) setTimeout(() => { vectors[body.project] = 3; Object.assign(jobs[body.project], { state: 'completed', processed: 3, remaining: 0, finished_unix_seconds: now, message: 'Task indexing completed.' }); }, 200);
          return send(200, indexStatus(body.project));
        }
        case '/api/settings/ai/v1/index/cancel': cancellations++; Object.assign(jobs[body.project], { state: 'cancelled', message: 'Indexing cancelled.', finished_unix_seconds: now }); return send(200, indexStatus(body.project));
        default: return send(404, { message: `Unmocked path ${url.pathname}` });
      }
    }
    let file = path.resolve(dist, `.${decodeURIComponent(url.pathname)}`);
    if (file !== dist && !file.startsWith(dist + path.sep)) return send(403, {});
    if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) file = path.join(dist, 'index.html');
    const mime = { '.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.svg': 'image/svg+xml' }[path.extname(file)] || 'application/octet-stream';
    res.writeHead(200, { 'Content-Type': mime, 'Cache-Control': 'no-store' }); fs.createReadStream(file).pipe(res);
  } catch (error) { console.error(error); if (!res.headersSent) send(500, { message: String(error) }); else res.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const base = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true, ...(process.env.TASKS_CHROME_PATH ? { executablePath: process.env.TASKS_CHROME_PATH } : {}) });
const page = await browser.newPage({ viewport: { width: 1440, height: 1100 }, timezoneId: 'Asia/Dubai' });
page.on('pageerror', error => errors.push(String(error)));
await page.route('**/*', route => route.request().url().startsWith(base) ? route.continue() : route.abort());
let stage = 'providers';
try {
  await page.goto(`${base}/settings/ai-providers`);
  const embeddings = page.getByRole('region', { name: 'Embeddings settings' });
  await embeddings.getByLabel('Enabled', { exact: true }).check();
  await embeddings.getByLabel('Embeddings endpoint').fill('https://embedding.example.test/v1/embeddings');
  await embeddings.getByLabel('Model', { exact: true }).fill('fixture-embedding-v1');
  await embeddings.getByLabel('API key (optional for local providers)', { exact: true }).fill(secret);
  assert.equal(await embeddings.getByRole('button', { name: 'Test connection', exact: true }).isDisabled(), true);
  await embeddings.getByRole('button', { name: 'Save', exact: true }).click();
  await embeddings.getByText('Settings saved.', { exact: true }).waitFor();
  assert.equal(await embeddings.getByLabel('API key (optional for local providers)', { exact: true }).inputValue(), '');
  await embeddings.getByRole('button', { name: 'Test connection', exact: true }).click();
  await embeddings.getByText(/Synthetic provider connection verified/).waitFor();
  await embeddings.getByLabel('Model', { exact: true }).fill('fixture-embedding-v2');
  await embeddings.getByRole('button', { name: 'Save', exact: true }).click();
  await embeddings.getByText('Settings saved.', { exact: true }).waitFor();
  assert.equal(keys.get('embeddings'), secret, 'editing a model keeps the stored key');

  const jev = page.getByRole('region', { name: 'Jev decisions settings' });
  await jev.getByLabel('Enabled', { exact: true }).check();
  await jev.getByLabel('API key', { exact: true }).fill(secret);
  await jev.getByRole('button', { name: 'Save', exact: true }).click();
  await jev.getByText('Settings saved.', { exact: true }).waitFor();
  await jev.getByRole('button', { name: 'Test connection', exact: true }).click();
  await jev.getByText(/Synthetic provider connection verified/).waitFor();
  assert.equal(await jev.getByLabel('API key', { exact: true }).inputValue(), '');
  await page.screenshot({ path: path.join(results, 'ai-providers.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'provider forms fit a narrow viewport');
  await page.screenshot({ path: path.join(results, 'ai-providers-mobile.png'), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1100 });

  stage = 'knowledge';
  await page.getByRole('link', { name: 'Knowledge & indexing', exact: true }).click();
  await page.getByLabel('Use project knowledge', { exact: true }).check();
  await page.getByLabel('Connected repository', { exact: true }).selectOption('source');
  await page.getByLabel('Wiki folders or files', { exact: true }).fill('docs/wiki');
  await page.getByLabel('Graphify source', { exact: true }).selectOption('upload');
  await page.getByRole('button', { name: 'Save sources', exact: true }).click();
  await page.getByText(/Sources saved/).waitFor();
  await page.getByRole('button', { name: 'Refresh sources', exact: true }).click();
  await page.getByText('Knowledge sources refreshed.', { exact: true }).waitFor();
  await page.getByLabel('Try a knowledge query', { exact: true }).fill('Valkey queues');
  await page.getByRole('button', { name: 'Preview context', exact: true }).click();
  await page.getByText('Queue architecture', { exact: true }).waitFor();
  const graph = { built_at_commit: commit, nodes: [{ id: 'x', label: 'Queue', source_file: 'src/queue.rs', source_location: 'L1' }], links: [] };
  await page.getByLabel('Upload Graphify export', { exact: true }).setInputFiles({ name: 'graph.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(graph)) });
  await page.getByText('Graph export saved. Check source status and commit below.', { exact: true }).waitFor();
  assert.equal(graphUploads, 1);
  await page.getByRole('button', { name: 'Preview context', exact: true }).click();
  await page.getByText('Queue architecture', { exact: true }).waitFor();

  stage = 'index';
  await page.getByRole('button', { name: 'Start indexing', exact: true }).click();
  await page.getByText('Task indexing completed.', { exact: true }).waitFor({ timeout: 15000 });
  assert.equal(vectors.DEMO, 3); assert.equal(indexStarts, 1);
  slowIndex = true;
  await page.getByLabel('Rebuild all vectors', { exact: true }).check();
  await page.getByRole('button', { name: 'Start indexing', exact: true }).click();
  await page.getByRole('button', { name: 'Cancel indexing', exact: true }).click();
  await page.getByText('Indexing cancelled.', { exact: true }).waitFor();
  assert.equal(cancellations, 1);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: path.join(results, 'knowledge-indexing.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'knowledge controls fit a narrow viewport');
  await page.screenshot({ path: path.join(results, 'knowledge-mobile.png'), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1100 });

  stage = 'project isolation';
  await page.getByLabel('Project', { exact: true }).selectOption('OTHER');
  await page.getByLabel('Use project knowledge', { exact: true }).waitFor();
  assert.equal(await page.getByLabel('Use project knowledge', { exact: true }).isChecked(), false);
  assert.equal(await page.getByText('Queue architecture', { exact: true }).count(), 0);
  assert.equal(vectors.OTHER, 0);
  await page.getByLabel('Project', { exact: true }).selectOption('DEMO');
  await page.getByLabel('Use project knowledge', { exact: true }).waitFor();
  assert.equal(await page.getByLabel('Use project knowledge', { exact: true }).isChecked(), true);
  assert.ok(calls.filter(c => c.path.endsWith('/save') && c.path.includes('knowledge')).every(c => c.project === 'DEMO'));

  stage = 'key removal';
  await page.getByRole('link', { name: 'AI providers', exact: true }).click();
  const reloaded = page.getByRole('region', { name: 'Embeddings settings' });
  assert.equal(await reloaded.getByLabel('API key (optional for local providers)', { exact: true }).inputValue(), '');
  await reloaded.getByLabel('Enabled', { exact: true }).uncheck();
  await reloaded.getByLabel('Remove saved key on save', { exact: true }).check();
  await reloaded.getByRole('button', { name: 'Save', exact: true }).click();
  await reloaded.getByText('Settings saved.', { exact: true }).waitFor();
  assert.equal(keys.has('embeddings'), false); assert.equal(settings.embeddings.enabled, false);

  stage = 'non-admin'; admin = false;
  await page.reload();
  await page.getByText('Admins only.', { exact: true }).waitFor();
  assert.equal(await page.getByLabel('Embeddings endpoint', { exact: true }).count(), 0);
  assert.deepEqual(errors, []);
  const report = { passed: ['provider save/test', 'stored credential never filled into the UI', 'key retained during model edits', 'Jev configuration', 'knowledge save/refresh/preview',
    'Graphify raw upload', 'index progress', 'cancellation', 'project isolation', 'key removal and disable', 'non-admin access', 'responsive settings'], saves, probes, graphUploads, indexStarts, cancellations, pageErrors: errors };
  fs.writeFileSync(path.join(results, 'report.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
} catch (error) {
  await page.screenshot({ path: path.join(results, 'failure.png'), fullPage: true }).catch(() => {});
  fs.writeFileSync(path.join(results, 'failure.txt'), `${stage}\n${error}\n${JSON.stringify(errors)}\n${await page.locator('body').innerText().catch(() => '')}`);
  throw error;
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
