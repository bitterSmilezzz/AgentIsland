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
    globalThis.location = { search: '?shell=island' };
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
window.__TAURI__ = {
  core: { invoke: async (command, args) => {
    if (command === 'get_boot_args') return {};
    if (command === 'get_settings') return { dock_edge: 'top', appearance: 'light' };
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
