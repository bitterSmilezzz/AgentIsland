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
      documentElement: mk(), body: mk(), getElementById: () => mk(), createElement: tag => { const node=mk(); if(tag==='dialog')node.querySelector=()=>mk(); return node; },
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
const placements = [];
const startup = process.env.TEST_COLD_NAVIGATION === '1';
let drains = 0;
let shows = 0;
const workbench = process.env.TEST_NAVIGATION_SHELL === 'workbench';
window.__TAURI__ = {
  core: { invoke: async (command, args) => {
    if (command === 'get_boot_args') return {};
    if (command === 'get_settings') return { dock_edge: 'top', appearance: 'light' };
    if (command === 'drain_navigation') {
      assert.ok(listeners.has('deeplink://navigate'), 'subscription must precede replay');
      drains++;
      if (startup && drains === 1) return workbench ? ['Workbench'] : ['Expand', 'Agent("codex")'];
      if (startup && !workbench && drains === 2) return ['Collapse'];
      return [];
    }
    if (command === 'show_workbench') shows++;
    if (command === 'place_island') placements.push(args);
    return null;
  } },
  event: { listen: async (event, callback) => { listeners.set(event, callback); return () => {}; } },
};
try {
  const { getState } = await import('../app/ui/js/main.js');
  for (let i = 0; i < 100 && !listeners.has('deeplink://navigate'); i++) {
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.ok(listeners.has('deeplink://navigate'), 'island boot must subscribe to deep links');
  const expectedDrains = startup ? (workbench ? 2 : 3) : 1;
  for (let i = 0; i < 100 && drains < expectedDrains; i++) {
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.equal(drains, expectedDrains, 'boot must consume all startup batches');
  if (workbench) {
    const send = action => listeners.get('deeplink://navigate')({ payload: { action } });
    assert.equal(shows, startup ? 1 : 0, 'cold workbench intent must reveal the window');
    await send('Analytics');
    assert.equal(getState().route, 'tokenAnalytics');
    const priorShows = shows;
    await send('Workbench');
    assert.equal(shows, priorShows + 1, 'workbench intent must reveal the window');
    assert.equal(getState().route, 'tokenAnalytics', 'reopening must preserve the current page');
    assert.equal(getState().workbenchPage, 'tokenAnalytics');
    const { hydrateReport } = await import('../app/ui/js/views.js');
    const body = dataset => ({
      isConnected: true, innerHTML: '', dataset: {}, contains:()=>false, setAttribute:noop, querySelector:selector=>selector==='[data-usage-trend]'?{querySelector:()=>({innerHTML:''}),querySelectorAll:()=>[]}:null,
      closest: selector => selector === '[data-page]' ? { dataset } : null,
    });
    const overview = body({ page: 'tokenAnalytics' });
    const analysis = body({ page: 'tokenAnalytics' });
    const detail = body({ page: 'agentDetail', agentId: 'codex' });
    const bodies = [overview, analysis, detail];
    document.querySelectorAll = selector => selector === '[data-report-root]' ? bodies : [];
    // 第一个在线工具没有明细，但聚合报告和指定详情都有数据。
    getState().engine = { snapshots: [{ id: 'antigravity' }] };
    const requests = [];
    const originalInvoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command, args) => {
      if (command !== 'get_report') return originalInvoke(command, args);
      requests.push(args.agentId);
      if (args.agentId === 'antigravity') return null;
      return {
        usage: { tokens24h: 9876, tokens_total: 20000, cost24h: 0, cost_total: 0, cost_estimated: false },
        hourly30d: [],
        models24h: [{ model: args.agentId ? 'detail-model' : 'aggregate-model', tokens: 9876, cost: 0 }],
      };
    };
    await hydrateReport();
    assert.deepEqual(requests, ['', '', 'codex'], 'analytics must request the aggregate, details their own agent');
    assert.ok(overview.innerHTML.includes('aggregate-model'));
    assert.ok(analysis.innerHTML.includes('aggregate-model'));
    assert.ok(detail.innerHTML.includes('detail-model'));
    getState().engine.snapshots = [];
    await hydrateReport();
    assert.equal(requests.at(-3), '', 'aggregate selection must not depend on online snapshots');
    console.log('PASS: workbench cold/live navigation and current page retention');
    process.exit(0);
  }
  if (startup) {
    assert.equal(getState().expanded, false);
    assert.equal(getState().route, 'list');
    assert.deepEqual(placements.map(size => size.width), [88, 330, 88]);
  }
  const send = action => listeners.get('deeplink://navigate')({ payload: { action } });
  await send('Expand');
  assert.equal(getState().expanded, true);
  assert.equal(placements.at(-1).width, 330);
  await send('Collapse');
  assert.equal(getState().expanded, false);
  assert.deepEqual(placements.at(-1), { width: 88, height: 17 });
  await send('Toggle');
  assert.equal(getState().expanded, true);
  await send('Toggle');
  assert.equal(getState().expanded, false);
  await Promise.all([send('Expand'), send('Collapse')]);
  assert.equal(getState().expanded, false);
  assert.equal(placements.at(-1).width, 88, 'rapid navigation must finish in arrival order');
  await send('Analytics');
  assert.equal(getState().expanded, true);
  assert.equal(getState().route, 'tokenAnalytics');
  await send('Agent("codex")');
  assert.equal(getState().route, 'agentDetail:codex');
  await send('Agent(invalid)');
  assert.equal(getState().route, 'agentDetail:codex');
  await send('Unknown');
  assert.equal(getState().route, 'agentDetail:codex');
  await send('Collapse');
  assert.equal(getState().route, 'list');
  const { agentIdFromIntent } = await import('../app/ui/js/navigation.js');
  assert.equal(agentIdFromIntent('Agent(' + JSON.stringify('agent"id)') + ')'), 'agent"id)');
  assert.equal(agentIdFromIntent('Agent(42)'), null);
  console.log('PASS: island boot, deep-link window transitions and routes');
  process.exit(0);
} catch (error) {
  console.error(error);
  process.exit(1);
}
