import assert from 'node:assert/strict';

const noop = () => {};
const mk = () => ({
  style: {}, dataset: {}, innerHTML: '', children: [], scrollTop: 0,
  classList: { contains: () => false, add: noop, remove: noop, toggle: noop },
  setAttribute: noop, removeAttribute: noop, addEventListener: noop,
  appendChild: noop, querySelectorAll: () => [],
  querySelector: selector => ['button', 'input', '[role=listbox]'].includes(selector) ? mk() : null,
  getBoundingClientRect: () => ({ width: 0, height: 0 }), focus: noop, remove: noop,
});
const root = mk(), sidebarBody = mk();
root.querySelector = selector => selector === '.sb-body' ? sidebarBody : null;
globalThis.document = {
  documentElement: mk(), body: mk(), getElementById: () => root, createElement: mk,
  querySelector: () => null, querySelectorAll: () => [], addEventListener: noop,
  createTextNode: mk,
};
globalThis.window = { innerWidth: 1120, innerHeight: 760, devicePixelRatio: 1 };
globalThis.location = { search: '?shell=workbench' };
globalThis.addEventListener = noop;
globalThis.removeEventListener = noop;
globalThis.matchMedia = () => ({ matches: false, addEventListener: noop, removeEventListener: noop });
globalThis.requestAnimationFrame = f => setTimeout(f, 0);
globalThis.cancelAnimationFrame = clearTimeout;
globalThis.localStorage = { getItem: () => null, setItem: noop, removeItem: noop };
globalThis.getComputedStyle = () => ({});
globalThis.innerWidth = 1120; globalThis.innerHeight = 760;
Object.defineProperty(globalThis, 'navigator', { value: { clipboard: {} }, configurable: true });
window.__TAURI__ = {
  core: { invoke: async command => {
    if (command === 'get_settings') return { dock_edge: 'top', appearance: 'light' };
    if (command === 'get_boot_args') return {};
    if (command === 'drain_navigation') return [];
    if (command === 'window_is_visible') return true;
    return null;
  } },
  event: { listen: async () => noop },
};

try {
  const { getState } = await import('../app/ui/js/main.js');
  const views = await import('../app/ui/js/views.js');
  await new Promise(resolve => setTimeout(resolve, 30));
  const state = getState();
  state.route = 'list'; state.workbenchPage = 'overview';
  const zero = { snapshots: [], grand_total: { tokens24h: 0, tokens_total: 0 }, latest_event: null };
  for (const render of [views.renderWorkbench, views.renderCard, views.renderSidebar]) {
    state.engine = null; root.dataset = {};
    render();
    assert.match(root.innerHTML, /等待采样/, `${render.name}: missing first snapshot must be unknown`);
    assert.doesNotMatch(root.innerHTML, /暂无在线智能体|全部正常|0 在线|<strong>0<\/strong>/,
      `${render.name}: unknown must not claim measured zero or health`);
    state.engine = zero; root.dataset = {};
    render();
    assert.match(root.innerHTML, /暂无在线智能体/, `${render.name}: measured empty is distinct`);
    assert.doesNotMatch(root.innerHTML, /等待采样/, `${render.name}: successful snapshot clears waiting`);
    assert.match(root.innerHTML, /<strong>0<\/strong>/, `${render.name}: measured zero stays zero`);
    state.engine = { ...zero, grand_total: { tokens24h: 987, tokens_total: 987 } }; root.dataset = {};
    render();
    assert.match(root.innerHTML, /<strong>987<\/strong>/, `${render.name}: real usage replaces placeholder`);
  }
  // The push path must replace the initial placeholder without rebuilding the
  // entire workbench, and must also handle a return to an unknown state.
  const monitor = mk(), summary = mk(), status = mk();
  document.querySelector = selector => ({ '[data-wb-monitor]': monitor,
    '[data-wb-summary]': summary, '[data-wb-status]': status })[selector] ?? null;
  state.engine = zero;
  views.renderWorkbenchMonitorOnly();
  assert.match(monitor.innerHTML, /暂无在线智能体/);
  assert.match(summary.innerHTML, /<strong>0<\/strong>/);
  state.engine = null;
  views.renderWorkbenchMonitorOnly();
  assert.match(monitor.innerHTML, /等待采样/);
  assert.doesNotMatch(summary.innerHTML, /<strong>0<\/strong>/);
  assert.equal(status.textContent, '等待采样');
  document.querySelector = () => null;
  // Independent functionality remains reachable while sampling is pending.
  state.workbenchPage = 'tokenAnalytics'; root.dataset = {};
  views.renderWorkbench();
  assert.match(root.innerHTML, /data-report-format/);
  state.workbenchPage = 'settings'; root.dataset = {};
  views.renderWorkbench();
  assert.match(root.innerHTML, /data-set=/);
  console.log('✓ 首次采样：三形态未知/零/实际用量、局部更新与独立页面通过');
  process.exit(0);
} catch (error) {
  console.error(error);
  process.exit(1);
}
