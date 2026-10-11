import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, resolve, extname, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

// Supply the existing browser runtime; this regression installs no dependencies.
// TEST_BROWSER_MODULE=/absolute/path/to/playwright/index.mjs node scripts/test-task-reader-leave.mjs
const { chromium } = await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
function fixture() {
  window.fixtureData = { revision: 1, projects: [{ id: 'project', name: 'Project' }],
    tasks: [{ id: 'task', title: 'Task', current_run_id: 'run', archived_ms: null }],
    runs: [{ id: 'run', task_id: 'task', status: 'ready', started_ms: 0 }], attentions: [],
    artifacts: [{ id: 'artifact', run_id: 'run', kind: 'result', title: 'Result', event_ref: 'a'.repeat(64) }] };
  window.readReplies = []; window.snapshotReplies = []; window.commands = [];
  window.transient = false; window.deferRead = false; window.deferSnapshot = false;
  window.__TAURI__ = { core: { invoke: async (command, args) => {
    commands.push({ command, args });
    if (command === 'get_boot_args') return { route: 'tasks' };
    if (command === 'get_settings') return { appearance: 'light', dock_edge: 'top', collapse_delay: 60, sample_interval: 2 };
    if (command === 'get_engine_state') return { snapshots: [], grand_total: { tokens24h: 0, tokens_total: 0, cost24h: 0, cost_total: 0 } };
    if (command === 'tasks_attention_summary') return { total: 0, human_count: 0, failed_count: 0 };
    if (command === 'tasks_snapshot') return deferSnapshot ? new Promise(resolve => snapshotReplies.push(resolve)) : structuredClone(fixtureData);
    if (command === 'task_sources' || command === 'drain_navigation') return [];
    if (command === 'window_is_visible') return true;
    if (command === 'todos_list') return { items: [], pending: 0 };
    if (command === 'task_artifact_content') {
      if (deferRead) return new Promise((resolve, reject) => readReplies.push({ resolve, reject }));
      return { available: true, text: 'ARTIFACT PRIVATE BODY', notice: 'Source record', ...(transient ? { transient_valid_for_ms: 60000 } : {}) };
    }
    return null;
  } }, event: { listen: async () => () => {} } };
  const schedule = window.setTimeout, cancel = window.clearTimeout;
  window.pendingTimeouts = new Map();
  window.setTimeout = (fn, delay, ...args) => {
    const id = schedule(() => { pendingTimeouts.delete(id); fn(...args); }, delay);
    pendingTimeouts.set(id, delay); return id;
  };
  window.clearTimeout = id => { pendingTimeouts.delete(id); cancel(id); };
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
const failures = []; let passed = 0;
async function check(name, work) {
  try { await work(); passed++; console.log(`PASS: ${name}`); }
  catch (error) { failures.push(name); console.error(`FAIL: ${name}\n${error.stack}`); }
}
async function prepare(reduced = false) {
  const page = await browser.newPage({ reducedMotion: reduced ? 'reduce' : 'no-preference' });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${origin}/fixture.html?shell=workbench`);
  await page.waitForFunction(() => document.querySelector('[data-tasks-root]')?.dataset.pageReady === 'true');
  await page.evaluate(() => { document.querySelector('[data-task-select]').click(); document.querySelector('.task-history').open = true; });
  return { page, errors };
}
async function read(page, transient = false) {
  await page.evaluate(transient => { window.transient = transient; document.querySelector('[data-task-artifact-read]').click(); }, transient);
  await page.waitForFunction(() => document.querySelector('[data-artifact-text]')?.textContent === 'ARTIFACT PRIVATE BODY');
}
try {
  browser = await chromium.launch({ headless: true, ...(process.env.TEST_CHROMIUM_PATH ? { executablePath: process.env.TEST_CHROMIUM_PATH } : {}) });
  for (const reduced of [false, true]) for (const transient of [false, true]) for (const destination of ['settings', 'todo']) {
    await check(`${transient ? 'temporary' : 'ordinary'} reader leaves for ${destination}, reduced=${reduced}`, async () => {
      const { page, errors } = await prepare(reduced);
      try {
        await read(page, transient);
        const result = await page.evaluate(async destination => {
          const root = document.getElementById('root'), taskRoot = root.querySelector('[data-tasks-root]');
          const body = taskRoot.querySelector('[data-artifact-text]'), viewer = body.closest('[data-task-artifact-view]');
          const drafts = ['[data-task-create] [name="title"]', '[data-task-project] [name="name"]', '[data-task-edit] [name="title"]', '[data-task-progress] [name="artifactTitle"]'];
          for (const [i, selector] of drafts.entries()) { const input = taskRoot.querySelector(selector); input.value = `unsaved ${i}`; input.dispatchEvent(new Event('input', { bubbles: true })); }
          const { renderWorkbench } = await import('/app/ui/js/views.js');
          renderWorkbench();
          const samePage = body.textContent;
          const cloneNode = Element.prototype.cloneNode, cloneBodies = [], cacheBodies = [], focusCalls = [];
          Element.prototype.cloneNode = function(...args) { if (this.contains(body)) cloneBodies.push({ text: body.textContent, hidden: viewer.hidden, notice: viewer.querySelector('[data-artifact-notice]').textContent }); return cloneNode.apply(this, args); };
          const { PageCache } = await import('/app/ui/js/page-host.js');
          const remember = PageCache.prototype.remember;
          PageCache.prototype.remember = function(key, node) { if (node?.contains(body)) cacheBodies.push(body.textContent); return remember.call(this, key, node); };
          const caller = taskRoot.querySelector('[data-task-artifact-read]'), focus = caller.focus;
          caller.focus = function(...args) { focusCalls.push('reader'); return focus.apply(this, args); };
          window.deferSnapshot = true;
          const nav = destination === 'todo' ? root.querySelector('[data-wb-task-view][data-wb-nav="todo"]') : root.querySelector('.wb-nav-item[data-wb-nav="settings"]');
          nav.click();
          const left = { body: body.textContent, hidden: viewer.hidden, bodyHidden: body.hidden, notice: viewer.querySelector('[data-artifact-notice]').textContent,
            outgoing: [...root.querySelectorAll('[data-page-outgoing]')].map(node => node.textContent), focus: document.activeElement.dataset.wbNav,
            timers: [...pendingTimeouts.values()].filter(delay => delay >= 50000 || delay === 5000).length };
          root.querySelector('.wb-nav-item[data-wb-nav="tasks"]').click();
          return { samePage, left, cloneBodies, cacheBodies, focusCalls, sameRoot: root.querySelector('[data-tasks-root]') === taskRoot,
            returnedBody: body.textContent, drafts: drafts.map(selector => taskRoot.querySelector(selector).value) };
        }, destination);
        assert.equal(result.samePage, 'ARTIFACT PRIVATE BODY', 'same-route render must preserve the reader');
        assert.equal(result.left.body, '', 'leave clears the retained original synchronously');
        assert.equal(result.left.hidden, true); assert.equal(result.left.bodyHidden, true); assert.equal(result.left.notice, '');
        assert.ok(result.left.outgoing.every(text => !text.includes('ARTIFACT PRIVATE BODY')), 'animation snapshots contain no plaintext');
        assert.ok(result.cloneBodies.every(row => row.text === '' && row.hidden && row.notice === ''), 'clear precedes cloneNode');
        if (!reduced) assert.equal(result.cloneBodies.length, 1, 'exercise actual snapshot path');
        assert.deepEqual(result.cacheBodies, [''], 'clear precedes PageCache.remember');
        assert.equal(result.left.focus, destination); assert.deepEqual(result.focusCalls, [], 'leave never restores reader focus');
        assert.equal(result.left.timers, 0, 'leave disposes temporary lifetime timers');
        assert.equal(result.sameRoot, true); assert.equal(result.returnedBody, '');
        assert.deepEqual(result.drafts, ['unsaved 0', 'unsaved 1', 'unsaved 2', 'unsaved 3']);
        assert.deepEqual(errors, []);
      } finally { await page.close(); }
    });
  }
  for (const reject of [false, true]) await check(`pending ${reject ? 'error' : 'content'} after immediate leave/return`, async () => {
    const { page, errors } = await prepare();
    try {
      const result = await page.evaluate(async reject => {
        const root = document.getElementById('root'), taskRoot = root.querySelector('[data-tasks-root]');
        const caller = taskRoot.querySelector('[data-task-artifact-read]'), body = taskRoot.querySelector('[data-artifact-text]'), viewer = body.closest('[data-task-artifact-view]');
        const input = taskRoot.querySelector('[data-task-edit] [name="title"]'); input.value = 'pending draft'; input.dispatchEvent(new Event('input', { bubbles: true }));
        window.deferRead = true; window.deferSnapshot = true;
        const pending = caller.onclick();
        root.querySelector('.wb-nav-item[data-wb-nav="settings"]').click();
        root.querySelector('.wb-nav-item[data-wb-nav="tasks"]').click();
        if (reject) readReplies[0].reject(new Error('LATE ERROR')); else readReplies[0].resolve({ available: true, text: 'LATE PRIVATE BODY', notice: 'LATE NOTICE', transient_valid_for_ms: 60000 });
        await pending;
        return { body: body.textContent, hidden: viewer.hidden, notice: viewer.querySelector('[data-artifact-notice]').textContent,
          busy: viewer.hasAttribute('aria-busy'), disabled: caller.disabled, focus: document.activeElement.dataset.wbNav,
          sameRoot: root.querySelector('[data-tasks-root]') === taskRoot, draft: input.value,
          timers: [...pendingTimeouts.values()].filter(delay => delay >= 50000 || delay === 5000).length };
      }, reject);
      assert.equal(result.body, '', 'late reply must not resurrect plaintext on the reconnected cached page');
      assert.equal(result.notice, '', 'late error/notice must not reopen the reader'); assert.equal(result.hidden, true);
      assert.equal(result.busy, false); assert.equal(result.disabled, false); assert.equal(result.focus, 'tasks');
      assert.equal(result.sameRoot, true); assert.equal(result.draft, 'pending draft'); assert.equal(result.timers, 0); assert.deepEqual(errors, []);
    } finally { await page.close(); }
  });
  await check('latest detail cleanup and explicit close focus', async () => {
    const { page, errors } = await prepare();
    try {
      await read(page, true);
      await page.evaluate(async () => { const { refreshTasks } = await import('/app/ui/js/tasks-page.js'); refreshTasks(); });
      await page.waitForFunction(() => document.querySelector('[data-task-artifact-view]')?.hidden);
      await page.evaluate(() => { document.querySelector('.task-history').open = true; });
      await read(page, true);
      const result = await page.evaluate(() => {
        const root = document.getElementById('root'), caller = root.querySelector('[data-task-artifact-read]'), body = root.querySelector('[data-artifact-text]');
        root.querySelector('[data-artifact-close]').click();
        const closedFocus = document.activeElement === caller, closedText = body.textContent;
        caller.click(); return { closedFocus, closedText };
      });
      assert.equal(result.closedFocus, true); assert.equal(result.closedText, '');
      await page.waitForFunction(() => document.querySelector('[data-artifact-text]')?.textContent === 'ARTIFACT PRIVATE BODY');
      const left = await page.evaluate(() => { const body = document.querySelector('[data-artifact-text]'); document.querySelector('.wb-nav-item[data-wb-nav="settings"]').click(); return { text: body.textContent, hidden: body.closest('[data-task-artifact-view]').hidden }; });
      assert.equal(left.text, '', 'leave handler follows the latest detail render'); assert.equal(left.hidden, true); assert.deepEqual(errors, []);
    } finally { await page.close(); }
  });
} finally {
  await browser?.close(); await new Promise(resolve => server.close(resolve));
}
console.log(JSON.stringify({ passed, failed: failures.length, failures }));
if (failures.length) process.exitCode = 1;
