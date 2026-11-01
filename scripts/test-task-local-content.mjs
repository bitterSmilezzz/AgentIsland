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
  window.lastArtifactVersion = null; window.lastRestore = null; window.artifactBodies = new Map();
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
      const artifactId = 'local-artifact';
      fixtureData.artifacts.push({ id: artifactId, run_id: 'run', kind: args.kind ?? 'result_review',
        title: args.artifactTitle, revision: 1, ...(hasBody ? { local_content: { sha256: 'a'.repeat(64), bytes: new TextEncoder().encode(args.artifactContent).length } } : {}) });
      if (hasBody) artifactBodies.set(artifactId, args.artifactContent);
      const snapshot = structuredClone(fixtureData);
      return deferProgress ? await new Promise(resolve => { progressReply = () => resolve(snapshot); }) : snapshot;
    }
    if (command === 'task_record_artifact_version') {
      if (args.expectedRevision !== fixtureData.revision) throw { code: 'stale_revision', message: '任务已更新，请刷新后重试' };
      lastArtifactVersion = args;
      fixtureData.revision++;
      const target = fixtureData.artifacts.find(item => item.id === args.artifactId);
      const logicalId = target.logical_id ?? target.id;
      const chain = fixtureData.artifacts.filter(item => (item.logical_id ?? item.id) === logicalId);
      for (const item of chain) item.logical_id = logicalId;
      const version = Math.max(...chain.map(item => item.revision)) + 1;
      const artifactId = `local-artifact-v${version}`;
      fixtureData.artifacts.push({ id: artifactId, logical_id: logicalId, run_id: target.run_id, kind: target.kind,
        title: args.artifactTitle, revision: version, local_content: { sha256: String(version).repeat(64).slice(0, 64), bytes: new TextEncoder().encode(args.artifactContent).length } });
      artifactBodies.set(artifactId, args.artifactContent);
      return structuredClone(fixtureData);
    }
    if (command === 'task_restore_artifact_version') {
      if (args.expectedRevision !== fixtureData.revision) throw { code: 'stale_revision', message: '任务已更新，请刷新后重试' };
      lastRestore = args;
      fixtureData.revision++;
      const target = fixtureData.artifacts.find(item => item.id === args.artifactId);
      const logicalId = target.logical_id ?? target.id;
      const chain = fixtureData.artifacts.filter(item => (item.logical_id ?? item.id) === logicalId);
      for (const item of chain) item.logical_id = logicalId;
      const version = Math.max(...chain.map(item => item.revision)) + 1;
      const artifactId = `local-artifact-v${version}`;
      fixtureData.artifacts.push({ id: artifactId, logical_id: logicalId, run_id: target.run_id, kind: target.kind,
        title: target.title, revision: version, local_content: { sha256: String(version).repeat(64).slice(0, 64), bytes: new TextEncoder().encode(artifactBodies.get(target.id)).length } });
      artifactBodies.set(artifactId, artifactBodies.get(target.id));
      return structuredClone(fixtureData);
    }
    if (command === 'task_artifact_content') {
      return { available: true, text: artifactBodies.get(args.artifactId) ?? localBody, notice: '本地保存 · 只读查看。' };
    }
    if (command === 'task_artifact_compare') {
      const from = fixtureData.artifacts.find(item => item.id === args.fromArtifactId);
      const to = fixtureData.artifacts.find(item => item.id === args.toArtifactId);
      return { fromRevision: from.revision, toRevision: to.revision, titleFrom: from.title, titleTo: to.title,
        titleChanged: from.title !== to.title, crossRun: from.run_id !== to.run_id,
        contentDiff: `--- v${from.revision}\n+++ v${to.revision}\n-${artifactBodies.get(from.id)}\n+${artifactBodies.get(to.id)}` };
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

  await page.locator('[data-task-artifact-read]').first().click();
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
  await page.locator('.task-version-chain').evaluate(node => { node.open = true; });
  const versionForm = page.locator('[data-task-version-form]');
  await versionForm.locator('[name="artifactTitle"]').fill('本地交付修订');
  const secondBody = '修订后的本地内容\n<img src=x onerror="window.injected=true">';
  await versionForm.locator('[name="artifactContent"]').fill(secondBody);
  await versionForm.locator('button[type="submit"]').click();
  await page.waitForFunction(() => window.lastArtifactVersion?.artifactContent === '修订后的本地内容\n<img src=x onerror="window.injected=true">');
  assert.deepEqual(await page.evaluate(() => lastArtifactVersion), {
    id: 'task', artifactId: 'local-artifact', artifactTitle: '本地交付修订', artifactContent: secondBody, expectedRevision: 2,
  });

  await page.locator('.task-version-chain').evaluate(node => { node.open = true; });
  const staleVersionForm = page.locator('[data-task-version-form]');
  await staleVersionForm.locator('[name="artifactTitle"]').fill('未丢失的草稿');
  await staleVersionForm.locator('[name="artifactContent"]').fill('等待刷新后再提交');
  await page.evaluate(() => { fixtureData.revision++; });
  await staleVersionForm.locator('button[type="submit"]').click();
  await page.waitForFunction(() => document.querySelector('[data-task-version-form] [data-task-form-feedback]')?.textContent.includes('草稿已保留'));
  assert.equal(await page.locator('[data-task-version-form] [name="artifactContent"]').inputValue(), '等待刷新后再提交');
  await page.locator('[data-task-refresh]').click();
  await page.waitForFunction(() => commands.filter(x => x.command === 'tasks_snapshot').length >= 2);
  await page.locator('[data-task-version-form] [data-task-discard]').click();
  await page.waitForFunction(() => document.querySelector('[data-task-version-form] [name="artifactContent"]')?.value === '');

  await page.locator('.task-version-chain').evaluate(node => { node.open = true; });
  const compareForm = page.locator('[data-task-version-compare]');
  await compareForm.locator('[name="fromArtifactId"]').selectOption('local-artifact');
  await compareForm.locator('[name="toArtifactId"]').selectOption('local-artifact-v2');
  await compareForm.locator('button[type="submit"]').click();
  await page.waitForFunction(() => document.querySelector('[data-artifact-text]')?.textContent.includes('+修订后的本地内容'));
  assert.equal(await page.locator('[data-artifact-title]').textContent(), '版本比较');
  assert.equal(await page.locator('[data-artifact-text] img').count(), 0, '比较结果必须以纯文本展示');
  assert.deepEqual(await page.evaluate(() => commands.find(x => x.command === 'task_artifact_compare').args), {
    id: 'task', fromArtifactId: 'local-artifact', toArtifactId: 'local-artifact-v2', expectedRevision: 4,
  });
  await page.locator('[data-artifact-close]').click();
  assert.equal(await page.locator('[data-artifact-text]').textContent(), '');

  await page.locator('.task-version-chain').evaluate(node => { node.open = true; });
  await page.locator('[data-task-artifact-restore="local-artifact"]').click();
  await page.waitForFunction(() => window.lastRestore?.artifactId === 'local-artifact');
  assert.deepEqual(await page.evaluate(() => lastRestore), { id: 'task', artifactId: 'local-artifact', expectedRevision: 4 });
  assert.equal(await page.evaluate(() => artifactBodies.get('local-artifact-v3')), submittedBody, '恢复必须复制所选历史正文并追加新版本');
  assert.deepEqual(errors, []);
  console.log('PASS: local task outputs append, compare, and restore immutable versions');
  await page.close();
} finally {
  await browser?.close(); await new Promise(resolve => server.close(resolve));
}
