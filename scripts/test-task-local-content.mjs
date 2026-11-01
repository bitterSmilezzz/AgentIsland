import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, resolve, extname, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const { chromium } = await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
function fixture() {
  window.fixtureData = { revision: 1, projects: [{ id: 'project', name: 'Project' }],
    tasks: [{ id: 'task', title: 'Task', current_run_id: 'run', archived_ms: null }],
    runs: [{ id: 'run', task_id: 'task', status: 'ready', started_ms: 0 }], attentions: [], artifacts: [] };
  window.commands = []; window.lastProgress = null; window.deferProgress = false; window.progressReply = null;
  window.localBody = '本地用户记录\n<img src=x onerror="window.injected=true">';
  window.__TAURI__ = { core: { invoke: async (command, args) => {
    commands.push({ command, args });
    if (command === 'get_boot_args') return { route: 'tasks' };
    if (command === 'get_settings') return { appearance: 'light', dock_edge: 'top', collapse_delay: 60, sample_interval: 2 };
    if (command === 'get_engine_state') return { snapshots: [], grand_total: { tokens24h: 0, tokens_total: 0, cost24h: 0, cost_total: 0 } };
    if (command === 'tasks_attention_summary') return { total: 0, human_count: 0, failed_count: 0 };
    if (command === 'tasks_snapshot') return structuredClone(fixtureData);
    if (command === 'task_sources' || command === 'drain_navigation') return [];
    if (command === 'window_is_visible') return true;
    if (command === 'todos_list') return { items: [], pending: 0 };
    if (command === 'task_record_progress') {
      lastProgress = args;
      fixtureData.revision++;
      const hasBody = Boolean(args.artifactContent?.trim());
      fixtureData.runs[0].status = args.status;
      fixtureData.artifacts.push({ id: 'local-artifact', run_id: 'run', kind: args.kind ?? 'result_review',
        title: args.artifactTitle, revision: 1, ...(hasBody ? { local_content: { sha256: 'a'.repeat(64), bytes: new TextEncoder().encode(args.artifactContent).length } } : {}) });
      const snapshot = structuredClone(fixtureData);
      return deferProgress ? await new Promise(resolve => { progressReply = () => resolve(snapshot); }) : snapshot;
    }
    if (command === 'task_artifact_content') {
      return { available: true, text: localBody, notice: '本地保存 · 只读查看。' };
    }
    return null;
  } }, event: { listen: async () => () => {} } };
}
const html = `<!doctype html><html class="shell-workbench theme-light"><head><meta charset="utf-8">
<link rel="stylesheet" href="/app/ui/css/tokens.css"><link rel="stylesheet" href="/app/ui/css/workbench.css">
</head><body><div id="root"></div><script>(${fixture})();</script><script type="module" src="/app/ui/js/main.js"></script></body></html>`;
const uiRoot = resolve(repository, 'app/ui');
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    if (pathname === '/fixture.html') { response.setHeader('Content-Type', 'text/html'); response.end(html); return; }
    if (!pathname.startsWith('/app/ui/')) { response.writeHead(403).end(); return; }
    const path = resolve(uiRoot, pathname.slice('/app/ui/'.length));
    if (!path.startsWith(uiRoot + sep)) { response.writeHead(403).end(); return; }
    response.setHeader('Content-Type', ({ '.js': 'text/javascript', '.css': 'text/css' })[extname(path)] || 'text/plain');
    response.end(await readFile(path));
  } catch { response.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.TEST_CHROMIUM_PATH ? { executablePath: process.env.TEST_CHROMIUM_PATH } : {}) });
  const page = await browser.newPage();
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${origin}/fixture.html?shell=workbench`);
  await page.waitForFunction(() => document.querySelector('[data-tasks-root]')?.dataset.pageReady === 'true');
  await page.evaluate(() => { document.querySelector('[data-task-select]').click(); document.querySelector('.task-history').open = true; });

  const form = page.locator('[data-task-progress]');
  await form.locator('[name="progress"]').selectOption('ready');
  await form.locator('[name="artifactTitle"]').fill('本地交付');
  await form.locator('[name="artifactContent"]').fill('撤销草稿');
  assert.equal(await form.locator('[data-task-discard]').isVisible(), true);
  await form.locator('[data-task-discard]').click();
  assert.equal(await form.locator('[name="artifactContent"]').inputValue(), '');

  await form.locator('[name="progress"]').selectOption('ready');
  await form.locator('[name="artifactTitle"]').fill('本地交付');
  const submittedBody = '本地用户记录\n<img src=x onerror="window.injected=true">';
  await form.locator('[name="artifactContent"]').fill(submittedBody);
  await page.evaluate(() => { window.deferProgress = true; });
  await form.locator('button[type="submit"]').click();
  await page.waitForFunction(() => window.lastProgress?.artifactContent === window.localBody);
  assert.equal(await form.locator('[name="artifactContent"]').isDisabled(), true, '正文不得在保存期间继续修改');
  await page.evaluate(() => window.progressReply());
  await page.waitForSelector('[data-task-artifact-read]');
  await page.locator('.task-history').evaluate(node => { node.open = true; });
  assert.deepEqual(await page.evaluate(() => ({
    status: lastProgress.status,
    title: lastProgress.artifactTitle,
    body: lastProgress.artifactContent,
    revision: lastProgress.expectedRevision,
    textareaDisabled: document.querySelector('[data-task-progress] textarea').disabled,
    readLabel: document.querySelector('[data-task-artifact-read]').textContent,
  })), { status: 'ready', title: '本地交付', body: submittedBody, revision: 1, textareaDisabled: false, readLabel: '查看产出' });

  await page.locator('[data-task-artifact-read]').click();
  await page.waitForFunction(() => document.querySelector('[data-artifact-text]')?.textContent === window.localBody);
  assert.equal(await page.locator('[data-artifact-title]').textContent(), '本地产出');
  assert.equal(await page.locator('[data-artifact-notice]').textContent(), '本地保存 · 只读查看。');
  assert.equal(await page.locator('[data-artifact-text] img').count(), 0, '正文必须以纯文本展示');
  assert.equal(await page.evaluate(() => window.injected), undefined, '正文不得执行 HTML');
  await page.locator('[data-artifact-close]').click();
  assert.equal(await page.locator('[data-artifact-text]').textContent(), '');
  assert.deepEqual(await page.evaluate(() => commands.filter(x => x.command === 'task_artifact_content').map(x => x.args)), [
    { id: 'task', runId: 'run', artifactId: 'local-artifact', expectedRevision: 2 },
  ]);
  assert.deepEqual(errors, []);
  console.log('PASS: local task output is saved, read as text, and cleared when closed');
  await page.close();
} finally {
  await browser?.close(); await new Promise(resolve => server.close(resolve));
}
