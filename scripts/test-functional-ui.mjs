import assert from 'node:assert/strict';

    const noop = () => {};
    const mk = () => ({
      className: 'shell-island', style: {}, dataset: {}, children: [], innerHTML: '',
      classList: { contains: () => false, add: noop, remove: noop, toggle: noop },
      appendChild: noop, setAttribute: noop, removeAttribute: noop, addEventListener: noop,
      querySelector: (selector) => selector === '#sliver' ? mk() : null, querySelectorAll: () => [],
      getBoundingClientRect: () => ({ width: 0, height: 0 }), focus: noop, remove: noop,
    });
    globalThis.document = {
      documentElement: mk(), body: mk(), getElementById: () => mk(), createElement: mk,
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
const root = mk();
document.getElementById = () => root;
window.__TAURI__ = {
  core: { invoke: async (command, args) => {
    if (command === 'get_settings') return { ...settings };
    if (command === 'get_boot_args') return {};
    if (command === 'drain_navigation') return [];
    if (command === 'remote_recent') return [{ title: 'Fixture', text: '未发：总开关已关闭' }];
    if (command === 'remote_secret_set') return { kind: 'ok' };
    if (command === 'remote_status') return { kind: 'smtpEmail', secretName: 'remote.smtpEmail',
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
  await listeners.get('settings://changed')({ payload: { ...settings, cpu_threshold: 23 } });
  assert.equal(getState().settings.cpu_threshold, 23, 'other-window threshold update must arrive');
  const { hydrateRemote, bindSettings, agentRowModel } = await import('../app/ui/js/views.js');
  assert.equal(agentRowModel({ token_usage: { tokens24h:0 } }).tokensText, '0', 'measured zero must differ from missing usage');
  assert.equal(agentRowModel({ token_usage:null }).tokensText, '—');
  const out = mk(), remoteRoot = mk(), input = { value: 'fixture-input' };
  const secretButton = mk();
  let saveSecret;
  secretButton.addEventListener = (_, handler) => { saveSecret = handler; };
  document.querySelector = selector => ({ '[data-remote-root]': remoteRoot, '[data-remote-out]': out,
    '[data-remote-save-secret]': secretButton, '[data-remote-secret]': input })[selector] ?? null;
  await hydrateRemote();
  assert.ok(remoteRoot.innerHTML.includes('remote.smtpEmail'), 'real camelCase status key must render');
  assert.ok(remoteRoot.innerHTML.includes('fixture-warning'), 'server readiness warnings must render');
  await saveSecret();
  assert.equal(input.value, '', 'successful lower-case ok result must clear the secret input');
  assert.ok(out.innerHTML.includes('密钥已写入'), 'keychain success must not be shown as a refusal');
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
  console.log('PASS: shared thresholds, partial saves, rollback, remote DTO and keychain feedback');
  process.exit(0);
} catch (error) { console.error(error); process.exit(1); }
