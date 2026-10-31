import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, readdir } from 'node:fs/promises';
import { resolve, extname, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
const { chromium } = await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const UI_DIR = 'app/ui';

// 这条测试覆盖状态机测不到的**真实 DOM**：按钮的显隐、进度文案、迟到进度事件。
// 状态机（scripts/test-update-state.mjs）证明逻辑正确，这里证明接线正确——
// 「按钮在错误的状态下仍然可见」这种问题只有真 DOM 能抓。

// ── 不变量：update-panel 调用的每个命令都已注册 ────────────────────────────────
const mainRs = await readFile(new URL('../app/src-tauri/src/main.rs', import.meta.url), 'utf8');
const handlerBlock = mainRs.slice(
  mainRs.indexOf('tauri::generate_handler!['),
  mainRs.indexOf('])', mainRs.indexOf('tauri::generate_handler![')));
const registered = new Set(
  handlerBlock.split('\n')
    .map(line => line.split('//')[0].trim().replace(/,$/, ''))
    .filter(Boolean)
    .map(name => name.split('::').pop()));
const uiDir = new URL('../app/ui/js/', import.meta.url);
const viewsSource = await readFile(new URL('../app/ui/js/views.js', import.meta.url), 'utf8');
const invoked = new Set();
for (const file of (await readdir(uiDir)).filter(f => f.endsWith('.js'))) {
  const source = await readFile(new URL(file, uiDir), 'utf8');
  for (const match of source.matchAll(/invoke\(\s*'([a-z0-9_]+)'/g)) invoked.add(match[1]);
}
const updateCommands = ['update_check', 'update_install', 'update_restart', 'update_session_status', 'update_dismiss_rejection', 'update_current_version'];
for (const command of updateCommands) {
  assert.ok(invoked.has(command), `update 面板没有调用 ${command}`);
  assert.ok(registered.has(command), `${command} 未在 invoke_handler 注册——界面会静默降级`);
}
assert.match(viewsSource, /querySelector\('\[data-update-panel\]'\)\?\.dispatchEvent\(new Event\('update:resume'\)\)/,
  '恢复缓存的 Settings 页面必须重新绑定更新状态监控');

// ── DOM：按钮的显隐就是状态机的投影 ──────────────────────────────────────────
// 必须加载**真实样式表**：`.mini-btn` 的 `display: inline-flex` 会盖过 UA 的
// `[hidden]{display:none}`，不加载 CSS 时按钮显隐断言全部成立，真机上却四个按钮同时显示。
// 这条测试存在的意义就是抓住「样式相关」的失效，所以它自己不能离开样式。
const FIXTURE_PAGE = `<!DOCTYPE html><html lang="zh-Hans" data-theme="dark"><head><meta charset="utf-8">
<link rel="stylesheet" href="/app/ui/css/tokens.css">
<link rel="stylesheet" href="/app/ui/css/island.css">
<link rel="stylesheet" href="/app/ui/css/sidebar.css">
<link rel="stylesheet" href="/app/ui/css/panels.css">
<link rel="stylesheet" href="/app/ui/css/workbench.css">
<link rel="stylesheet" href="/app/ui/css/agents.css">
</head><body>
<script type="module">
  import { updatePanelHtml, bindUpdatePanel } from '/app/ui/js/update-panel.js';
  document.body.innerHTML = updatePanelHtml();
  window.__update = bindUpdatePanel(document);
</script></body></html>`;

const server = createServer(async (req, res) => {
  try {
    if ((req.url || '').startsWith('/update-panel-fixture.html')) {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      res.end(FIXTURE_PAGE);
      return;
    }
    const url = new URL(req.url, 'http://x').pathname;
    const rel = url.startsWith('/app/ui/') ? url.slice('/app/ui'.length) : url;
    // 刻意用 `join` + **相对** UI_DIR：这个仓库路径含中文，macOS 上目录名是 NFD
    // 而 URL 传的是 NFC，`resolve` 出的绝对路径会让 readFile 报 ENOENT——症状是
    // 「文件明明在，模块却 404」，很容易误判成导入写错。
    const path = join(UI_DIR, decodeURIComponent(rel));
    const body = await readFile(path);
    res.writeHead(200, { 'content-type': extname(path) === '.js' ? 'text/javascript' : 'text/html; charset=utf-8' });
    res.end(body);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise(r => server.listen(0, '127.0.0.1', r));

const browser = await chromium.launch();
try {
  const progressEvents = [];
  let installCalls = 0, restartCalls = 0, checkCalls = 0;
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.addInitScript(() => {
    window.__fixture = {
      // 夹具数据必须放在页面里：写在 Node 侧的变量不会跨进 addInitScript 的闭包，
      // 结果 invoke 拿到 undefined、面板走到 failed 分支，症状是「点了没反应」。
      checkResult: { currentVersion: '0.0.326', available: true, version: '0.0.327', notes: '修复若干问题', date: '2026-10-19' },
      sessionStatus: { phase: 'idle', reason: null },
      progressListeners: [], emitProgress(downloaded, total) {
      for (const fn of window.__fixture.progressListeners) fn({ payload: { downloaded, total } });
    } };
    window.__TAURI__ = {
      core: { invoke: async (cmd) => {
        if (cmd === 'update_current_version') return '0.0.326';
        if (cmd === 'update_session_status') return window.__fixture.sessionStatus;
        if (cmd === 'update_check') { window.__fixture.checkCalls = (window.__fixture.checkCalls || 0) + 1; return window.__fixture.checkResult; }
        if (cmd === 'update_install') {
          window.__fixture.installCalls = (window.__fixture.installCalls || 0) + 1;
          window.__fixture.sessionStatus = { phase: 'downloading', reason: null };
          // 挂起直到测试放行：install 立刻 resolve 的话 phase 会从 downloading
          // 直接跳到 ready，中间态采样不到，进度文案就没被真正验证过。
          window.__fixture.phaseAtRestart = document.querySelector('[data-update-panel]')?.dataset.updatePhase;
          await new Promise(resolve => { window.__fixture.releaseInstall = resolve; });
          window.__fixture.sessionStatus = { phase: 'ready', reason: null };
          return null;
        }
        if (cmd === 'update_restart') {
          window.__fixture.restartCalls = (window.__fixture.restartCalls || 0) + 1;
          window.__fixture.restartedAt = window.__fixture.phaseAtRestart;
          return null;
        }
        if (cmd === 'update_dismiss_rejection') { window.__fixture.sessionStatus = { phase: 'idle', reason: null }; return null; }
        return null;
      } },
      event: { listen: async (name, handler) => {
        if (name === 'update://progress') { window.__fixture.progressListeners.push(handler); return () => {}; }
        return () => {};
      } },
    };
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await page.waitForFunction(() => window.__update != null, null, { timeout: 10000 });
  await page.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'idle', null, { timeout: 5000 });
  assert.deepEqual(errors, [], '更新面板挂载不得抛异常');

  // 加载**真实样式表**，让显隐断言在真机同款环境下成立（本项目 index.html 共 6 张）。
  // 少了这一步，测试就在「没有作者样式」的条件下判断显隐，结论未必能外推。
  assert.ok(await page.evaluate(() => document.styleSheets.length) >= 3,
    '样式表没有加载，本轮的显隐断言不可信');

  const visible = async key => await page.locator(`[data-update-${key}]`).isVisible();
  const statusText = async () => await page.locator('[data-update-status]').textContent();

  // 初始：只有「检查更新」可见。安装/重启入口**不存在**——不是 disabled，是不出现。
  try {
    await page.waitForFunction(() => document.querySelector('[data-update-current]')?.textContent?.includes('0.0.326'), null, { timeout: 5000 });
  } catch (e) {
    console.log('DIAG current=', JSON.stringify(await page.locator('[data-update-current]').textContent()));
    console.log('DIAG phase=', await page.evaluate(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase));
    console.log('DIAG errors=', JSON.stringify(errors));
    console.log('DIAG tauri=', await page.evaluate(() => typeof window.__TAURI__ + '/' + typeof window.__TAURI__?.core?.invoke));
    console.log('DIAG raw=', await page.evaluate(() => window.__TAURI__.core.invoke('update_current_version')));
    console.log('DIAG ctrlState=', await page.evaluate(() => JSON.stringify(window.__update?.state)));
    throw e;
  }
  assert.equal(await visible('check'), true, '初始必须给出检查入口');
  assert.equal(await visible('install'), false, '未检查时不得出现「下载并安装」');
  assert.equal(await visible('restart'), false, '未安装时不得出现「立即重启」');
  assert.match(await statusText(), /尚未检查更新/);
  assert.equal(await page.evaluate(() => window.__fixture.checkCalls || 0), 0, '**挂载不得自动检查**');

  // 检查到有更新：安装入口出现，重启入口仍不出现。
  await page.click('[data-update-check]');
  await page.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'available', null, { timeout: 5000 });
  assert.equal(await visible('install'), true, '有更新时必须给出安装入口');
  assert.equal(await visible('restart'), false, '还没安装，不得给出重启入口');
  assert.match(await statusText(), /发现新版本 v0\.0\.327/);
  assert.match(await page.locator('[data-update-notes]').textContent(), /修复若干问题/);

  // 下载进度：按钮变百分比，状态行给具体字节。
  await page.click('[data-update-install]');
  await page.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'downloading', null, { timeout: 5000 });
  await page.evaluate(() => window.__fixture.emitProgress(1024, 4096));
  // 事件形状与 Tauri 一致：`{event, id, payload}`，进度在 payload 里。
  await page.waitForFunction(() => /25%/.test(document.querySelector('[data-update-install]')?.textContent || ''), null, { timeout: 5000 });
  await page.evaluate(() => { window.__fixture.releaseInstall(); });
  await page.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'ready', null, { timeout: 5000 });
  assert.equal(await page.evaluate(() => window.__fixture.restartCalls || 0), 0, '**安装完成绝不能自动重启**');
  assert.equal(await visible('restart'), true, '装完必须给用户一个显式重启入口');
  assert.equal(await page.evaluate(() => window.__fixture.restartedAt ?? null), null, '在用户点击之前不得发出过重启');

  await page.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'ready', null, { timeout: 5000 });
  assert.equal(await page.evaluate(() => window.__fixture.installCalls), 1, '只安装一次');
  assert.equal(await page.evaluate(() => window.__fixture.restartCalls || 0), 0, '**安装完成绝不能自动重启**');
  assert.equal(await visible('restart'), true, '装完必须给用户一个显式重启入口');
  assert.equal(await page.evaluate(() => window.__fixture.restartedAt ?? null), null, '在用户点击之前不得发出过重启');
  assert.match(await statusText(), /重启后生效/);

  // 迟到进度不得把已完成的状态拽回下载中。
  await page.evaluate(() => window.__fixture.emitProgress(4096, 4096));
  await page.waitForTimeout(120);
  assert.equal(await page.evaluate(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase), 'ready');

  // 用户显式点击才重启。
  await page.click('[data-update-restart]');
  await page.waitForFunction(() => window.__fixture.restartCalls === 1, null, { timeout: 5000 });

  // 安装失败：显示原因，且**不得**出现重启入口。
  const failure = await browser.newPage();
  failure.on('pageerror', e => errors.push(e.message));
  await failure.addInitScript(() => {
    window.__fixture = { progressListeners: [], sessionStatus: { phase: 'idle', reason: null } };
    window.__TAURI__ = {
      core: { invoke: async (cmd) => {
        if (cmd === 'update_current_version') return '0.0.326';
        if (cmd === 'update_session_status') return window.__fixture.sessionStatus;
        if (cmd === 'update_check') return { currentVersion: '0.0.326', available: true, version: '0.0.327', notes: '', date: '' };
        if (cmd === 'update_install') { window.__fixture.sessionStatus = { phase: 'rejected', reason: '更新包签名校验失败，已拒绝安装' }; throw '更新包签名校验失败，已拒绝安装'; }
        if (cmd === 'update_dismiss_rejection') { window.__fixture.sessionStatus = { phase: 'idle', reason: null }; return null; }
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await failure.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await failure.waitForFunction(() => window.__update != null, null, { timeout: 10000 });
  await failure.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'idle', null, { timeout: 5000 });
  await failure.click('[data-update-check]');
  await failure.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'available', null, { timeout: 5000 });
  await failure.click('[data-update-install]');
  await failure.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'rejected', null, { timeout: 5000 });
  assert.match(await failure.locator('[data-update-status]').textContent(), /签名校验失败/, '必须把拒绝原因说给用户');
  assert.equal(await failure.locator('[data-update-restart]').isVisible(), false, '安装失败不得出现重启入口');
  assert.equal(await failure.locator('[data-update-check]').isVisible(), false, '签名拒绝不得显示重试入口');
  assert.equal(await failure.locator('[data-update-dismiss]').textContent(), '关闭', '签名拒绝使用关闭入口，不暗示可重试');
  // 离开并重新进入后仍要恢复拒绝状态；只有关闭说明才会清除后端拒绝门。
  const refused = await browser.newPage();
  refused.on('pageerror', e => errors.push(e.message));
  await refused.addInitScript(() => {
    window.__fixture = { dismissCalls: 0 };
    window.__TAURI__ = {
      core: { invoke: async cmd => {
        if (cmd === 'update_current_version') return '0.0.326';
        if (cmd === 'update_session_status') return { phase: 'rejected', reason: '更新包签名校验失败，已拒绝安装' };
        if (cmd === 'update_dismiss_rejection') { window.__fixture.dismissCalls += 1; return null; }
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await refused.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await refused.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'rejected', null, { timeout: 5000 });
  assert.equal(await refused.locator('[data-update-check]').isVisible(), false, '重进后仍不得提供签名拒绝的重试入口');
  await refused.click('[data-update-dismiss]');
  await refused.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'idle', null, { timeout: 5000 });
  assert.equal(await refused.evaluate(() => window.__fixture.dismissCalls), 1, '关闭说明必须清除后端拒绝门');

  // 检查阶段也可能收到签名错误；关闭拒绝说明后必须能再次手动检查。
  const checkRefusal = await browser.newPage();
  checkRefusal.on('pageerror', e => errors.push(e.message));
  await checkRefusal.addInitScript(() => {
    window.__fixture = { status: { phase: 'idle', reason: null }, dismissCalls: 0 };
    window.__TAURI__ = {
      core: { invoke: async cmd => {
        if (cmd === 'update_current_version') return '0.0.326';
        if (cmd === 'update_session_status') return window.__fixture.status;
        if (cmd === 'update_check') {
          const reason = '更新包签名校验失败，已拒绝安装';
          window.__fixture.status = { phase: 'rejected', reason };
          throw reason;
        }
        if (cmd === 'update_dismiss_rejection') {
          window.__fixture.dismissCalls += 1;
          window.__fixture.status = { phase: 'idle', reason: null };
          return null;
        }
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await checkRefusal.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await checkRefusal.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'idle', null, { timeout: 5000 });
  await checkRefusal.click('[data-update-check]');
  await checkRefusal.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'rejected', null, { timeout: 5000 });
  await checkRefusal.click('[data-update-dismiss]');
  await checkRefusal.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'idle', null, { timeout: 5000 });
  assert.equal(await checkRefusal.evaluate(() => window.__fixture.dismissCalls), 1, '检查阶段签名拒绝也必须可显式解除');

  // 普通安装失败跨页后仍显示脱敏原因，并保留明确的手动重试入口。
  const ordinaryFailure = await browser.newPage();
  ordinaryFailure.on('pageerror', e => errors.push(e.message));
  await ordinaryFailure.addInitScript(() => {
    window.__fixture = { sessionStatus: { phase: 'idle', reason: null } };
    window.__TAURI__ = {
      core: { invoke: async cmd => {
        if (cmd === 'update_current_version') return '0.0.326';
        if (cmd === 'update_session_status') return window.__fixture.sessionStatus;
        if (cmd === 'update_check') return { currentVersion: '0.0.326', available: true, version: '0.0.327', notes: '', date: '' };
        if (cmd === 'update_install') {
          const reason = '网络不可用，无法连接更新服务';
          window.__fixture.sessionStatus = { phase: 'failed', reason };
          throw reason;
        }
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await ordinaryFailure.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await ordinaryFailure.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'idle', null, { timeout: 5000 });
  await ordinaryFailure.click('[data-update-check]');
  await ordinaryFailure.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'available', null, { timeout: 5000 });
  await ordinaryFailure.click('[data-update-install]');
  await ordinaryFailure.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'failed', null, { timeout: 5000 });
  const failedAgain = await browser.newPage();
  failedAgain.on('pageerror', e => errors.push(e.message));
  await failedAgain.addInitScript(() => {
    window.__TAURI__ = {
      core: { invoke: async cmd => {
        if (cmd === 'update_current_version') return '0.0.326';
        if (cmd === 'update_session_status') return { phase: 'failed', reason: '网络不可用，无法连接更新服务' };
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await failedAgain.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await failedAgain.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'failed', null, { timeout: 5000 });
  assert.match(await failedAgain.locator('[data-update-status]').textContent(), /网络不可用/, '重进后保留具体脱敏失败原因');
  assert.equal(await failedAgain.locator('[data-update-check]').isVisible(), true, '普通失败后保留用户明确点击的重试入口');

  // 检查中离开设置页再进入，直到旧检查结束前不得开放并发检查。
  const checking = await browser.newPage();
  checking.on('pageerror', e => errors.push(e.message));
  await checking.addInitScript(() => {
    let status = { phase: 'checking', reason: null };
    setTimeout(() => { status = { phase: 'idle', reason: null }; }, 1200);
    window.__TAURI__ = {
      core: { invoke: async cmd => {
        if (cmd === 'update_current_version') return '0.0.326';
        if (cmd === 'update_session_status') return status;
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await checking.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await checking.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'checking', null, { timeout: 5000 });
  assert.equal(await checking.locator('[data-update-check]').isVisible(), false, '恢复检查中状态时不得提供第二次检查');
  await checking.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'idle', null, { timeout: 5000 });

  // 导航卸载面板时必须解绑进度监听，避免重复进入设置页后累积监听器。
  const detached = await browser.newPage();
  detached.on('pageerror', e => errors.push(e.message));
  await detached.addInitScript(() => {
    window.__fixture = { unlistenCalls: 0, status: { phase: 'idle', reason: null } };
    window.__TAURI__ = {
      core: { invoke: async cmd => {
        if (cmd === 'update_current_version') return '0.0.326';
        if (cmd === 'update_session_status') return window.__fixture.status;
        if (cmd === 'update_check') return { currentVersion: '0.0.326', available: true, version: '0.0.327', notes: '', date: '' };
        return null;
      } },
      event: { listen: async () => () => { window.__fixture.unlistenCalls += 1; } },
    };
  });
  await detached.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await detached.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'idle', null, { timeout: 5000 });
  await detached.click('[data-update-check]');
  await detached.waitForFunction(() => window.__update?.state.phase === 'available', null, { timeout: 5000 });
  await detached.evaluate(() => {
    window.__fixture.cachedPanel = document.querySelector('[data-update-panel]');
    window.__fixture.cachedPanel?.remove();
  });
  await detached.waitForFunction(() => window.__fixture.unlistenCalls === 1, null, { timeout: 5000 });
  await detached.evaluate(() => {
    window.__fixture.status = { phase: 'ready', reason: null };
    const panel = window.__fixture.cachedPanel;
    document.body.append(panel);
    panel?.dispatchEvent(new Event('update:resume'));
  });
  await detached.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'ready', null, { timeout: 5000 });
  assert.equal(await detached.locator('[data-update-restart]').isVisible(), true, '恢复缓存页面时重新读取全局待重启状态');

  // 旧挂载的轮询响应不得覆盖恢复后状态，也不得清掉新挂载的轮询器。
  const stale = await browser.newPage();
  stale.on('pageerror', e => errors.push(e.message));
  await stale.addInitScript(() => {
    window.__fixture = { status: { phase: 'checking', reason: null }, statusCalls: 0, resolveStale: null };
    window.__TAURI__ = {
      core: { invoke: async cmd => {
        if (cmd === 'update_current_version') return '0.0.326';
        if (cmd === 'update_session_status') {
          window.__fixture.statusCalls += 1;
          if (window.__fixture.statusCalls === 2) {
            return new Promise(resolve => { window.__fixture.resolveStale = resolve; });
          }
          return window.__fixture.status;
        }
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await stale.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await stale.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'checking', null, { timeout: 5000 });
  await stale.waitForFunction(() => window.__fixture.statusCalls === 2, null, { timeout: 5000 });
  await stale.evaluate(async () => {
    const panel = document.querySelector('[data-update-panel]');
    window.__fixture.cachedPanel = panel;
    panel?.remove();
    await new Promise(resolve => setTimeout(resolve, 0));
    document.body.append(panel);
    panel?.dispatchEvent(new Event('update:resume'));
  });
  await stale.waitForFunction(() => window.__fixture.statusCalls >= 3, null, { timeout: 5000 });
  await stale.evaluate(() => window.__fixture.resolveStale?.({ phase: 'idle', reason: null }));
  await stale.waitForTimeout(50);
  assert.equal(await stale.locator('[data-update-panel]').getAttribute('data-update-phase'), 'checking', '旧挂载的 idle 回包不得覆盖新检查状态');
  await stale.evaluate(() => { window.__fixture.status = { phase: 'idle', reason: null }; });
  await stale.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'idle', null, { timeout: 5000 });

  // 下载中离开并重新进入设置页，状态轮询必须等到安装完成再显示重启入口。
  const pending = await browser.newPage();
  pending.on('pageerror', e => errors.push(e.message));
  await pending.addInitScript(() => {
    window.__fixture = { progressListeners: [], restartCalls: 0 };
    let status = { phase: 'downloading', reason: null };
    setTimeout(() => { status = { phase: 'ready', reason: null }; }, 1500);
    window.__TAURI__ = {
      core: { invoke: async cmd => {
        if (cmd === 'update_current_version') return '0.0.326';
        if (cmd === 'update_session_status') return status;
        if (cmd === 'update_restart') { window.__fixture.restartCalls += 1; return null; }
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await pending.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await pending.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'downloading', null, { timeout: 5000 });
  await pending.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'ready', null, { timeout: 5000 });
  assert.equal(await pending.locator('[data-update-restart]').isVisible(), true, '重新进入设置页后仍须显示重启入口');
  assert.equal(await pending.locator('[data-update-check]').isVisible(), false, '待重启时不得再次检查更新');
  await pending.click('[data-update-restart]');
  await pending.waitForFunction(() => window.__fixture.restartCalls === 1, null, { timeout: 5000 });

  // 版本查询失败时也必须有一个可用的初始界面：不能因为读不到版本号就整块空白。
  const degraded = await browser.newPage();
  degraded.on('pageerror', e => errors.push(e.message));
  await degraded.addInitScript(() => {
    window.__TAURI__ = {
      core: { invoke: async (cmd) => {
        if (cmd === 'update_current_version') throw '读不到包信息';
        if (cmd === 'update_session_status') return { phase: 'idle', reason: null };
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await degraded.goto(`http://127.0.0.1:${server.address().port}/update-panel-fixture.html`);
  await degraded.waitForFunction(() => window.__update != null, null, { timeout: 10000 });
  await degraded.waitForFunction(() => document.querySelector('[data-update-panel]')?.dataset.updatePhase === 'idle', null, { timeout: 5000 });
  assert.equal(await degraded.locator('[data-update-check]').isVisible(), true, '版本读不到时仍要能点检查');
  assert.equal(await degraded.locator('[data-update-current]').textContent(), '', '读不到版本时不编一个版本号');
  assert.match(await degraded.locator('[data-update-status]').textContent(), /尚未检查更新/);

  assert.deepEqual(errors, [], `页面抛出异常：${errors.join(' | ')}`);
  console.log('PASS: update panel DOM — button visibility, progress, reentry state/reason restore, explicit refusal, no signature retry, listener cleanup');
} finally {
  await browser.close();
  server.close();
}
