import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, extname, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
const { chromium } = await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const UI_DIR = 'app/ui';

// 档位页的**真实 DOM 回归**。形状门禁（gw02.profileui）证明接线写在源码里；
// 这里证明用户真的操作时它对：表单回填、提交体带上推理档位与目录标识，
// 以及应用结果里的能力边界**逐字上屏**（漏显示就只剩「配置已写入」，容易被读成「模型能用了」）。
//
// `main.js` 用桩替换：它是整个应用的启动器，真实加载会去连 Tauri。
// 除它之外全部是真实模块，所以被测的是真代码。

const STUB_MAIN = `const state = { providerActiveName: '', workbenchPage: 'provider', route: 'list' };
export const getState = () => state;
export const setState = (patch) => Object.assign(state, patch ?? {});
export const saveSettings = async () => {};
export const expand = () => {};
export const collapse = () => {};
export const armCollapseTimer = () => {};
export const scheduleRender = () => {};
export const resizeToContent = () => {};
export const applyEdge = () => {};
export const applyLayout = () => {};
export const retainWorkbenchControls = () => {};
`;

const LIMITATIONS =
  '只修改本机 Codex 配置，不验证账号或模型可用性；目标接口需要兼容 Codex 的 API 协议。'
  + 'API key 不由本应用保存，档位只记环境变量名。配置目标不等于运行中的会话模型；'
  + '切换后请开启新会话，必要时重启 Codex。原生 profile 的覆盖配置暂不支持切换。';

const STATUS = {
  installed: true, config_path: '/fixture/config.toml', active_provider_id: 'acme',
  active_profile_id: 'work', profile_count: 1, configured_model: 'gpt-5',
  configured_provider: 'acme', current_draft: null, capture_notice: '', config_error: null,
  revision: 'r1', last_applied: null, drifted: false, record_error: null,
  limitations: LIMITATIONS, profiles: [], backups: [],
};

const PROFILES = [{
  id: 'work', name: '工作账号', model: 'gpt-5', provider_id: 'acme', provider_name: 'Acme',
  base_url: 'https://api.example.invalid/v1', env_key: 'ACME_API_KEY', wire_api: 'responses',
  reasoning_effort: 'high', catalog_id: 'claude-sonnet-4-5',
}];

// 保存后界面会用 provider_status 重新渲染，档位清单随状态一起给（真实 DTO 也是这个形状）。
STATUS.profiles = PROFILES;

const FIXTURE_PAGE = `<!DOCTYPE html><html lang="zh-Hans" data-theme="dark"><head><meta charset="utf-8">
<link rel="stylesheet" href="/app/ui/css/tokens.css">
<link rel="stylesheet" href="/app/ui/css/sidebar.css">
<link rel="stylesheet" href="/app/ui/css/panels.css">
</head><body><!-- 不放 .wb-overview：那是工作台概览分支，会只渲染紧凑摘要 -->
<section data-provider-root></section>
<script type="module">
  window.__calls = [];
  window.__TAURI__ = { core: { invoke: async (cmd, args) => {
    window.__calls.push([cmd, args]);
    if (cmd === 'provider_status') return window.__status;
    if (cmd === 'provider_list_profiles' || cmd === 'provider_profiles') return window.__profiles;
    if (cmd === 'provider_list_backups' || cmd === 'provider_backups') return [];
    if (cmd === 'provider_save_profile') return window.__profiles;
    if (cmd === 'provider_apply_profile') return window.__applied;
    throw new Error('unexpected command ' + cmd);
  } }, event: { listen: async () => () => {} } };
</script>
<script type="module">
  import { renderProviderPage } from '/app/ui/js/views.js';
  renderProviderPage(document.querySelector('[data-provider-root]'),
    window.__status, window.__profiles, [], [], true);
  window.__ready = true;
</script></body></html>`;

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://x').pathname;
    if (url.startsWith('/provider-fixture.html')) {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      res.end(FIXTURE_PAGE);
      return;
    }
    if (url === '/app/ui/js/main.js') {
      res.writeHead(200, { 'content-type': 'text/javascript' });
      res.end(STUB_MAIN);
      return;
    }
    const rel = url.startsWith('/app/ui/') ? url.slice('/app/ui'.length) : url;
    // 仓库路径含中文：目录名是 NFD 而 URL 是 NFC，必须 join 相对路径。
    const path = join(UI_DIR, decodeURIComponent(rel));
    const body = await readFile(path);
    res.writeHead(200, { 'content-type': extname(path) === '.js' ? 'text/javascript' : 'text/html; charset=utf-8' });
    res.end(body);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise(r => server.listen(0, '127.0.0.1', r));

const browser = await chromium.launch({
  headless: true,
  ...(process.env.TEST_CHROMIUM_PATH ? { executablePath: process.env.TEST_CHROMIUM_PATH } : {}),
});
try {
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.addInitScript(([status, profiles, limitations]) => {
    window.__status = status;
    window.__profiles = profiles;
    window.__applied = {
      config_path: '/fixture/config.toml',
      backup_name: 'config-2026.toml',
      record_warning: null,
      limitations,
    };
  }, [STATUS, PROFILES, LIMITATIONS]);
  await page.goto(`http://127.0.0.1:${server.address().port}/provider-fixture.html`);
  await page.waitForFunction(() => window.__ready === true);

  const form = page.locator('[data-provider-form]');
  const field = name => form.locator(`[data-field="${name}"]`);
  const calls = () => page.evaluate(() => window.__calls);

  // ── 回填：编辑既有档位时，推理档位与目录标识必须带进表单 ──────────────────
  await page.locator('[data-edit="work"]').click();
  assert.equal(await field('reasoning_effort').inputValue(), 'high', '推理档位未回填');
  assert.equal(await field('catalog_id').inputValue(), 'claude-sonnet-4-5', '目录标识未回填');
  assert.equal(await field('model').inputValue(), 'gpt-5', '模型标识未回填');

  // ── 提交体：两个新字段真的进了 payload ──────────────────────────────────
  // 高级字段默认折叠（编辑既有档位时不自作主张展开）：像真实用户那样先展开。
  await page.locator('details[data-provider-advanced] summary').click();
  await field('reasoning_effort').selectOption('medium');
  await field('catalog_id').fill('claude-sonnet-4-5-20250929');
  await page.locator('[data-save-profile]').click();
  await page.waitForFunction(() => window.__calls.some(([cmd]) => cmd === 'provider_save_profile'));
  const save = (await calls()).find(([cmd]) => cmd === 'provider_save_profile');
  assert.equal(save[1].profile.reasoning_effort, 'medium', '提交体丢了推理档位');
  assert.equal(save[1].profile.catalog_id, 'claude-sonnet-4-5-20250929', '提交体丢了目录标识');

  // ── 应用后的提示必须逐字带上能力边界 ────────────────────────────────────
  const rowsAfterSave = await page.locator('[data-profile-row]').count();
  const switchAfterSave = await page.locator('[data-switch]').count();
  assert.ok(switchAfterSave >= 1, `保存后档位行消失：rows=${rowsAfterSave} switches=${switchAfterSave}`);
  await page.locator('[data-switch="work"]').click();
  await page.locator('[data-yes]').click();
  await page.waitForFunction(() => document.querySelector('[data-toast]')?.textContent.includes('不验证账号或模型可用性'));
  const toast = await page.locator('[data-toast]').textContent();
  assert.ok(toast.includes('config-2026.toml'), `提示应包含备份名：${toast}`);
  // 断言「不许宣称可用」时要精确：边界文案里本来就有「模型可用性」这几个字，
  // 用宽松正则会把那句否定句本身判成违规（第一版就是这么误伤的）。
  assert.ok(toast.includes('不验证账号或模型可用性'), `提示缺少能力边界：${toast}`);
  assert.ok(!toast.includes('可以生成'), `提示不得宣称可以生成：${toast}`);
  assert.ok(!toast.includes('已验证可用'), `提示不得宣称已验证可用：${toast}`);

  assert.deepEqual(errors, [], '页面不得抛出未处理异常');
  console.log('PASS: provider page behaves in a real browser — backfill, payload fields and capability boundary verified');
} finally {
  await browser.close();
  server.close();
}
