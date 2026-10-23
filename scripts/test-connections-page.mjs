import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, extname, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
const { chromium } = await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const UI_DIR = 'app/ui';

// 连接页的**真实 DOM 回归**。形状门禁（gw01.refreshui）证明接线写对了；这里证明
// 用户真正操作时它 behave 得对：字段联动、提交体、回填与余额渲染——「提交时把
// 半份 refresh 送出去」「余额没查到却显示占位值」这类问题只有真 DOM 能抓。

const SNAPSHOT = {
  schema_version: 1,
  revision: 7,
  items: [{
    id: 'conn-refresh', kind: 'custom_endpoint', name: '声明网关', base_url: 'https://gw.example/',
    credential_ref: { kind: 'environment', name: 'FIXTURE_AUTH' },
    auth: { kind: 'bearer' },
    refresh: { token_url: 'https://gw.example/token', client_id: 'web-client', refresh_token_env: 'FIXTURE_REFRESH' },
    quota: { endpoint: 'api/user/quota', balance_path: 'data.remain_quota', unit: 'Tokens', plan_path: 'data.plan' },
    enabled: true, created_ms: 1, updated_ms: 2,
  }, {
    id: 'conn-plain', kind: 'custom_endpoint', name: '普通端点', base_url: 'http://127.0.0.1:11434/',
    credential_ref: null, auth: { kind: 'none' }, refresh: null, quota: null,
    enabled: true, created_ms: 1, updated_ms: 2,
  }],
};

const PROBE_WITH_BALANCE = {
  connection_id: 'conn-refresh', checked_at_ms: 42, status: 'readable', failure_code: null,
  service_version: null, reason: '服务状态可读；余额/用量已核实（1000000 Tokens）',
  models_verified: false, usage_verified: true,
  balance: { amount: 1000000, unit: 'Tokens', raw_text: '1000000 Tokens', plan: 'Team-Enterprise' },
};

const FIXTURE_PAGE = `<!DOCTYPE html><html lang="zh-Hans" data-theme="dark"><head><meta charset="utf-8">
<link rel="stylesheet" href="/app/ui/css/tokens.css">
<link rel="stylesheet" href="/app/ui/css/sidebar.css">
<link rel="stylesheet" href="/app/ui/css/panels.css">
</head><body>
<script type="module">
  window.__calls = [];
  window.__TAURI__ = { core: { invoke: async (cmd, args) => {
    window.__calls.push([cmd, args]);
    if (cmd === 'connections_list') return window.__snapshot;
    if (cmd === 'connection_save') {
      window.__saved = args.config;
      return { ...window.__snapshot, revision: window.__snapshot.revision + 1 };
    }
    if (cmd === 'connection_test') return window.__probe;
    throw new Error('unexpected command ' + cmd);
  } } };
</script>
<script type="module">
  import { pageConnections, hydrateConnections } from '/app/ui/js/connections-page.js';
  const root = document.getElementById('root');
  root.innerHTML = pageConnections();
  window.__ready = hydrateConnections(root);
</script>
<div id="root"></div>
</body></html>`;

const server = createServer(async (req, res) => {
  try {
    if ((req.url || '').startsWith('/connections-fixture.html')) {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      res.end(FIXTURE_PAGE);
      return;
    }
    const url = new URL(req.url, 'http://x').pathname;
    const rel = url.startsWith('/app/ui/') ? url.slice('/app/ui'.length) : url;
    // 仓库路径含中文：目录名是 NFD 而 URL 是 NFC，必须 join 相对路径（见 test-update-panel.mjs）。
    const path = join(UI_DIR, decodeURIComponent(rel));
    const body = await readFile(path);
    res.writeHead(200, { 'content-type': extname(path) === '.js' ? 'text/javascript' : 'text/html; charset=utf-8' });
    res.end(body);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise(r => server.listen(0, '127.0.0.1', r));

// 用 wrapper 选定的 Chromium（固定版本）；独立运行时退回包内默认。
const browser = await chromium.launch({
  headless: true,
  ...(process.env.TEST_CHROMIUM_PATH ? { executablePath: process.env.TEST_CHROMIUM_PATH } : {}),
});
try {
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.addInitScript(([snapshot, probe]) => {
    window.__snapshot = snapshot;
    window.__probe = probe;
  }, [SNAPSHOT, PROBE_WITH_BALANCE]);
  await page.goto(`http://127.0.0.1:${server.address().port}/connections-fixture.html`);
  await page.evaluate(() => window.__ready);
  const form = page.locator('form[data-connection-form]');
  const field = name => form.locator(`[name="${name}"]`);

  // ── 字段联动：真 DOM 上验证显隐 ────────────────────────────────────────────
  const hidden = async name => page.locator(`[name="${name}"]`).evaluate(el => el.closest('label').hidden);
  // 编辑器默认折叠（<details>）：与真实用户一样先展开。
  await page.locator('details[data-connection-editor] summary').click();
  assert.equal(await hidden('auth_header'), true, '未选 header 方案时请求头名必须隐藏');
  await field('auth_kind').selectOption('bearer');
  assert.equal(await hidden('auth_header'), true);
  assert.equal(await hidden('refresh_kind'), false, '有认证方案时刷新开关必须可见');
  await field('refresh_kind').selectOption('refresh_token');
  assert.equal(await hidden('refresh_token_url'), false);
  await field('refresh_kind').selectOption('none');
  assert.equal(await hidden('refresh_token_url'), true, '不刷新时三件套必须隐藏');
  await field('quota_kind').selectOption('declared');
  assert.equal(await hidden('quota_endpoint'), false);
  await field('quota_kind').selectOption('none');
  assert.equal(await hidden('quota_path'), true);
  // 留下一个「开关=刷新」的残留状态，用于下面的负例（hidden 上的 selectOption 不可交互）。
  await field('refresh_kind').selectOption('refresh_token');
  await field('auth_kind').selectOption('none');
  assert.equal(await hidden('refresh_kind'), true, '不发认证头时刷新开关必须隐藏');

  // ── 提交体（负例在前）：不发认证头时，即使刷新开关残留为刷新也必须送 null ──
  await field('name').fill('边界连接');
  await field('base_url').fill('https://edge.example/');
  await page.locator('[data-connection-save]').click();
  await page.waitForFunction(() => window.__saved);
  let saved = await page.evaluate(() => window.__saved);
  assert.equal(saved.auth.kind, 'none');
  assert.equal(saved.refresh, null, '不发认证头时必须整体不声明刷新');
  assert.equal(saved.quota, null, '未声明余额时必须送 null');

  // ── 提交体：完整三件套，环境变量名转大写 ───────────────────────────────────
  await page.evaluate(() => { window.__saved = null; });
  // 负例保存成功后 reset() 会折叠编辑器：像真实用户一样再次展开。
  await page.locator('details[data-connection-editor] summary').click();
  await field('name').fill('声明网关');
  await field('base_url').fill('https://edge.example/');
  await field('auth_kind').selectOption('bearer');
  await field('refresh_kind').selectOption('refresh_token');
  await field('refresh_token_url').fill('https://edge.example/token');
  await field('refresh_client_id').fill('web-client');
  await field('refresh_env').fill('edge_refresh');   // 故意小写
  await field('quota_kind').selectOption('declared');
  await field('quota_endpoint').fill('api/user/quota');
  await field('quota_path').fill('data.remain_quota');
  await field('quota_unit').fill('Tokens');
  await field('quota_plan_path').fill('data.plan');
  await page.locator('[data-connection-save]').click();
  await page.waitForFunction(() => window.__saved);
  saved = await page.evaluate(() => window.__saved);
  assert.deepEqual(saved.refresh, {
    token_url: 'https://edge.example/token', client_id: 'web-client', refresh_token_env: 'EDGE_REFRESH',
  }, '三件套必须整体提交且环境变量名转大写');
  assert.deepEqual(saved.quota, {
    endpoint: 'api/user/quota', balance_path: 'data.remain_quota', unit: 'Tokens',
    plan_path: 'data.plan',
  });

  // ── 编辑回填：既有连接的刷新与余额声明必须原样回到表单 ─────────────────────
  await page.locator('[data-connection-edit="conn-refresh"]').click();
  assert.equal(await field('refresh_token_url').inputValue(), 'https://gw.example/token');
  assert.equal(await field('refresh_env').inputValue(), 'FIXTURE_REFRESH');
  assert.equal(await field('quota_path').inputValue(), 'data.remain_quota');
  assert.equal(await field('quota_unit').inputValue(), 'Tokens');
  assert.equal(await field('quota_plan_path').inputValue(), 'data.plan', '套餐路径必须回填');
  await page.locator('[data-connection-cancel]').click();

  // ── 余额渲染：查到数值才显示，且显示的是回执原文；没查到的不显示占位 ────────
  assert.equal(await page.locator('.connection-balance').count(), 0, '读取前不得显示余额');
  await page.locator('[data-connection-test="conn-refresh"]').click();
  await page.waitForFunction(() => document.querySelectorAll('.connection-balance').length === 1);
  const balanceText = await page.locator('.connection-balance').first().textContent();
  assert.ok(balanceText.includes('1000000 Tokens'), `余额必须显示回执原文：${balanceText}`);
  assert.ok(balanceText.includes('Team-Enterprise'), `套餐必须按网关原样显示：${balanceText}`);
  assert.equal(await page.locator('[data-connection-test="conn-plain"]').count(), 1);
  await page.locator('[data-connection-test="conn-plain"]').click();
  await page.waitForTimeout(200);
  assert.equal(await page.locator('.connection-balance').count(), 1, '无余额读数的连接不得渲染占位');
  // 套餐同理：回执里没有套餐就不许出现「套餐」字样（不显示占位）。
  // 整页只应有一处「套餐」字样（conn-refresh 那条），其余连接不显示。
  // 只数**余额行里**的「套餐」：编辑表单里那个「套餐取值路径」标签不算渲染占位。
  const planMentions = await page.locator('.connection-balance', { hasText: '套餐' }).count();
  assert.equal(planMentions, 1, `没有套餐读数的连接不得渲染套餐占位（实际 ${planMentions} 处）`);

  assert.deepEqual(errors, [], '页面不得抛出未处理异常');
  console.log('PASS: connections page behaves in a real browser — linkage, submit payloads, backfill and balance rendering verified');
} finally {
  await browser.close();
  server.close();
}
