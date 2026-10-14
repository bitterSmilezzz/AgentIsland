import assert from 'node:assert/strict';

    const noop = () => {};
    const mk = () => ({
      className: 'shell-island', style: {}, dataset: {}, children: [], innerHTML: '',
      classList: { contains: () => false, add: noop, remove: noop, toggle: noop },
      appendChild: noop, setAttribute: noop, removeAttribute: noop, addEventListener: noop,
      querySelector: (selector) => ['#sliver', '.sb-body'].includes(selector) ? mk() : null, querySelectorAll: () => [],
      getBoundingClientRect: () => ({ width: 0, height: 0 }), focus: noop, remove: noop,
    });
    // workbench 壳会往 body 挂一个真实的快捷导航 <dialog>（app/ui/js/quick-navigation.js:11-13），
    // 随后用 dialog.querySelector('input' / 'button' / '[role=listbox]') 取它的部件并绑事件。
    // 所以 createElement 出来的节点要把「产品自己写进 innerHTML 的标记」解析成可查询的子节点：
    // 桩结构跟着产品 markup 走，产品改了 dialog 内部结构时这里不会停在旧形状上假装通过。
    const VOID_TAGS = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source', 'track', 'wbr']);
    const tokenRe = /<!--[\s\S]*?-->|<\/([a-zA-Z][\w-]*)\s*>|<([a-zA-Z][\w-]*)((?:"[^"]*"|'[^']*'|[^>"'])*?)(\/?)>/g;
    const attrRe = /([:@\w-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?/g;
    const simpleRe = /([a-zA-Z][\w-]*)|#([\w-]+)|\.([\w-]+)|\[([\w:-]+)(?:([~^$*|]?=)(?:"([^"]*)"|'([^']*)'|([^\]\s]+)))?\]/g;
    const attributesOf = (raw) => { const found = new Map(); attrRe.lastIndex = 0; let m;
      while ((m = attrRe.exec(raw))) found.set(m[1].toLowerCase(), m[2] ?? m[3] ?? m[4] ?? '');
      return found; };
    // 只认简单选择器（标记 / #id / .class / [属性] / [属性=值]）；带组合子的选择器返回 null，
    // 与改动前的桩一致 —— 行为断言归 test-quick-navigation.mjs 的真实桩，这里只保证壳能起来。
    const matchesSimple = (node, selector) => { simpleRe.lastIndex = 0; let m, matched = false;
      while ((m = simpleRe.exec(selector))) { matched = true;
        if (m[1]) { if (node.tagName !== m[1].toUpperCase()) return false; }
        else if (m[2]) { if (node.id !== m[2]) return false; }
        else if (m[3]) { if (!node.classList.contains(m[3])) return false; }
        else if (m[5] === '=') { if (node.getAttribute(m[4]) !== (m[6] ?? m[7] ?? m[8])) return false; }
        else if (m[5]) return false;
        else if (!node.hasAttribute(m[4])) return false; }
      return matched; };
    const findNode = (nodes, selector) => { for (const node of nodes) { if (matchesSimple(node, selector)) return node;
      const nested = findNode(node.children, selector); if (nested) return nested; } return null; };
    const collectNodes = (nodes, selector, found = []) => { for (const node of nodes) { if (matchesSimple(node, selector)) found.push(node);
      collectNodes(node.children, selector, found); } return found; };
    const parseMarkup = (html) => { const top = [], open = [{ children: top }]; tokenRe.lastIndex = 0; let m;
      while ((m = tokenRe.exec(html))) {
        if (m[0].startsWith('<!--')) continue;
        if (m[1]) { if (open.length > 1) open.pop(); continue; }
        const node = mkNode(m[2]);
        for (const [key, value] of attributesOf(m[3] ?? '')) { node.setAttribute(key, value);
          if (key === 'id') node.id = value;
          else if (key === 'class') node.className = value;
          else if (key.startsWith('data-')) node.dataset[key.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase())] = value; }
        open.at(-1).children.push(node);
        if (!m[4] && !VOID_TAGS.has(m[2].toLowerCase())) open.push(node);
      }
      return top; };
    const mkNode = (tag) => {
      const node = mk(), attrs = new Map(); let html = '';
      node.tagName = String(tag).toUpperCase(); node.children = []; node.className = '';
      node.classList = { contains: (name) => String(node.className).split(/\s+/).includes(name), add: noop, remove: noop, toggle: noop };
      node.setAttribute = (key, value) => { attrs.set(String(key).toLowerCase(), String(value ?? '')); };
      node.removeAttribute = (key) => { attrs.delete(String(key).toLowerCase()); };
      node.getAttribute = (key) => { const name = String(key).toLowerCase(); return attrs.has(name) ? attrs.get(name) : null; };
      node.hasAttribute = (key) => attrs.has(String(key).toLowerCase());
      node.querySelector = (selector) => findNode(node.children, String(selector));
      node.querySelectorAll = (selector) => collectNodes(node.children, String(selector));
      Object.defineProperty(node, 'innerHTML', { configurable: true, enumerable: true, get: () => html,
        set: (value) => { html = String(value); node.children = parseMarkup(html); } });
      return node;
    };
    const createdDialogs = [];
    globalThis.document = {
      documentElement: mk(), body: mk(), getElementById: () => mk(),
      createElement: (tag) => { const node = mkNode(tag); if (String(tag).toLowerCase() === 'dialog') createdDialogs.push(node); return node; },
      querySelector: () => null, querySelectorAll: () => [], addEventListener: noop,
      createTextNode: () => mk(),
    };
    globalThis.window = { innerWidth: 400, innerHeight: 800, devicePixelRatio: 2, getComputedStyle: () => ({}) };
    globalThis.location = { search: '?shell=' + (process.env.TEST_NAVIGATION_SHELL ?? 'island') };
    Object.defineProperty(globalThis, 'navigator', { value: { clipboard: { writeText: async () => {} } }, configurable: true });
    globalThis.addEventListener = noop; globalThis.removeEventListener = noop;
    globalThis.matchMedia = () => ({ matches: false, addEventListener: noop, removeEventListener: noop });
    globalThis.requestAnimationFrame = (f) => setTimeout(f, 0);
    globalThis.cancelAnimationFrame = (h) => clearTimeout(h);
    globalThis.localStorage = { getItem: () => null, setItem: noop, removeItem: noop };
    globalThis.innerWidth = 400; globalThis.innerHeight = 800;
    
globalThis.getComputedStyle = () => ({});

const listeners = new Map();
let settings = { dock_edge: 'top', appearance: 'light', cpu_threshold: 6,
  remote_kind: 'smtpEmail', remote_channels: { smtpEmail: {} }, remote_policy: { master_enabled: false } };
const patches = [];
let failSave = false;
const secretWrites = [];
let testSends = 0;
const root = mk();
document.getElementById = () => root;
window.__TAURI__ = {
  core: { invoke: async (command, args) => {
    if (command === 'get_settings') return { ...settings };
    if (command === 'get_boot_args') return {};
    if (command === 'window_is_visible') return true;
    if (command === 'get_engine_state') return { snapshots: [], grand_total: {tokens24h: 0}, marker: 'fresh' };
    if (command === 'drain_navigation') return [];
    if (command === 'remote_recent') return [{ title: 'Fixture', text: '未发：总开关已关闭' }];
    if (command === 'remote_secret_set') { secretWrites.push(args.value); return { kind: 'ok' }; }
    if (command === 'remote_send_test') { testSends++; return '平台已受理'; }
    if (command === 'remote_status') return { kind: settings.remote_kind, secretName: `remote.${settings.remote_kind}`,
      insecureEndpoint: 'fixture-warning', policy: settings.remote_policy, limitations: 'fixture' };
    if (command === 'patch_settings') {
      if (failSave) throw new Error('fixture-write-failed');
      patches.push(args.patch);
      settings = { ...settings, ...args.patch };
      return { ...settings };
    }
    return null;
  } }, event: { listen: async (event, callback) => { listeners.set(event, callback); return () => {}; } },
};
try {
  const { getState } = await import('../app/ui/js/main.js');
  for (let i=0; i<100 && !listeners.has('deeplink://navigate'); i++) await new Promise(resolve => setTimeout(resolve, 10));
  assert.ok(listeners.has('settings://changed'), 'every window must subscribe to shared settings');
  if (process.env.TEST_NAVIGATION_SHELL === 'workbench') {
    // 快捷导航只在 workbench 壳里挂（views.js renderWorkbench → bindQuickNavigation）。
    // 断言它的部件确实是从产品写进 dialog 的标记里解析出来的，桩不许退化成「任意选择器给个空壳」。
    const dialog = createdDialogs.at(-1);
    assert.ok(dialog, 'workbench 壳必须挂上快捷导航对话框');
    assert.equal(dialog.querySelector('input')?.id, 'quick-navigation-input');
    assert.equal(dialog.querySelector('[role=listbox]')?.id, 'quick-navigation-results');
    assert.equal(typeof dialog.querySelector('button')?.onclick, 'function', '关闭按钮必须解析出来并绑上 close');
  }
  await listeners.get('settings://changed')({ payload: { ...settings, cpu_threshold: 23 } });
  assert.equal(getState().settings.cpu_threshold, 23, 'other-window threshold update must arrive');
  const { hydrateRemote, bindSettings, agentRowModel } = await import('../app/ui/js/views.js');
  assert.equal(agentRowModel({ token_usage: { tokens24h:0 } }).tokensText, '0', 'measured zero must differ from missing usage');
  assert.equal(agentRowModel({ token_usage:null }).tokensText, '—');
  const out = mk(), remoteRoot = mk(), input = { value: 'fixture-input' };
  const secretButton = mk(), testButton = mk();
  let saveSecret, sendTest;
  testButton.addEventListener = (_, handler) => { sendTest = handler; };
  secretButton.addEventListener = (_, handler) => { saveSecret = handler; };
  document.querySelector = selector => ({ '[data-remote-root]': remoteRoot, '[data-remote-out]': out,
    '[data-remote-save-secret]': secretButton, '[data-remote-secret]': input, '[data-remote-test]': testButton })[selector] ?? null;
  await hydrateRemote();
  assert.ok(remoteRoot.innerHTML.includes('remote.smtpEmail'), 'real camelCase status key must render');
  assert.ok(remoteRoot.innerHTML.includes('fixture-warning'), 'server readiness warnings must render');
  await saveSecret();
  assert.equal(input.value, '', 'successful lower-case ok result must clear the secret input');
  assert.ok(out.innerHTML.includes('密钥已写入'), 'keychain success must not be shown as a refusal');
  assert.match(remoteRoot.innerHTML, /<details[^>]*data-remote-advanced>/, 'advanced defaults closed');
  for (const kind of ['feishuBot','wechatPushPlus','qqPushPlus','qqOneBot']) assert.ok(remoteRoot.innerHTML.includes(`value="${kind}"`));
  await saveSecret();
  assert.equal(secretWrites.length, 1, 'blank save must not delete a stored credential');
  await sendTest({ currentTarget:testButton });
  assert.equal(testSends, 0, 'disabled privacy gate must not send a test');
  getState().settings.remote_kind = settings.remote_kind = 'feishuBot';
  await hydrateRemote();
  input.value = 'https://open.feishu.cn/open-apis/bot/v2/hook/fixture-hook';
  await saveSecret();
  assert.deepEqual(JSON.parse(secretWrites.at(-1)), { webhook:'https://open.feishu.cn/open-apis/bot/v2/hook/fixture-hook', signingSecret:'' });
  assert.ok(!JSON.stringify(patches).includes('fixture-hook'), 'webhook must never enter settings');
  const notices = [];
  const row = { appendChild: node => { assert.equal(typeof node, 'object'); notices.push(node.textContent); } };
  let change;
  const control = { dataset: { set: 'cpu_threshold' }, type: 'number', tagName: 'INPUT', value: '24',
    closest: () => row, addEventListener: (_, handler) => { change = handler; } };
  const settingsRoot = { querySelectorAll: () => [control] };
  document.querySelector = selector => selector === '[data-settings-root]' ? settingsRoot : null;
  bindSettings();
  await change();
  assert.deepEqual(patches.at(-1), { cpu_threshold: 24 }, 'threshold saves must not overwrite another window’s settings');
  failSave = true; control.value = '30';
  await change();
  assert.equal(getState().settings.cpu_threshold, 24);
  assert.equal(control.value, 24);
  assert.ok(notices.at(-1).includes('fixture-write-failed'), 'save failure must be visible, without appendChild string errors');
  await listeners.get('ui://visibility')({ payload: false });
  assert.equal(getState().windowVisible, false);
  let hiddenDomReads = 0;
  const previousQuery = document.querySelectorAll;
  document.querySelectorAll = () => { hiddenDomReads++; return []; };
  await listeners.get('settings://changed')({ payload: { ...settings, cpu_threshold: 31 } });
  assert.equal(getState().settings.cpu_threshold, 31, 'hidden window keeps settings current');
  assert.equal(hiddenDomReads, 0, 'hidden settings events must not touch the DOM');
  document.querySelectorAll = previousQuery;
  await listeners.get('ui://visibility')({ payload: true });
  assert.equal(getState().engine.marker, 'fresh', 'reopening fetches current engine state');
  assert.equal(getState().windowVisible, true);
  console.log('PASS: shared thresholds, partial saves, rollback, remote DTO and keychain feedback');
  process.exit(0);
} catch (error) { console.error(error); process.exit(1); }
