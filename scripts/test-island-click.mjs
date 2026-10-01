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

const handlers = new Map();
const header = mk();
header.addEventListener = (event, handler) => handlers.set(event, handler);
const card = { ...mk(), offsetHeight: 240, getBoundingClientRect: () => ({ height: 240 }) };
const root = mk();
root.querySelector = selector => selector === '[data-drag]' ? header : selector === '.card' ? card : selector === '#sliver' ? mk() : null;
root.contains = node => node === header;
document.getElementById = () => root;
document.querySelector = selector => selector === '.card' ? card : null;
const calls = [];
window.__TAURI__ = {
 core: { invoke: async (command, args) => {
   calls.push({ command, args });
   if (command === 'get_boot_args') return {};
   if (command === 'get_settings') return { appearance: 'light', dock_edge: 'top' };
   if (command === 'drain_navigation') return [];
   if (command === 'snap_nearest_edge') return 'top';
   return null;
 } }, event: { listen: async () => () => {} },
};
try {
 const { setState } = await import('../app/ui/js/main.js');
 await new Promise(resolve => setTimeout(resolve, 20));
 setState({ expanded: true });
 const { renderCard } = await import('../app/ui/js/views.js');
 renderCard();
 const button = { closest: () => ({}) };
 const empty = { closest: () => null };
 const emit = (name, target, x=100, y=100) => handlers.get(name)?.({ button: 0, target, screenX:x, screenY:y });
 emit('mousedown', button); emit('mouseup', button);
 await new Promise(resolve => setTimeout(resolve, 90));
 assert.equal(calls.filter(c => c.command === 'snap_nearest_edge').length, 0, 'button click must not move or resize the island');
 emit('mousedown', empty); emit('mouseup', empty);
 await new Promise(resolve => setTimeout(resolve, 90));
 assert.equal(calls.filter(c => c.command === 'snap_nearest_edge').length, 0, 'plain header click must not snap');
 emit('mousedown', empty); emit('mouseup', empty, 130, 140);
 await new Promise(resolve => setTimeout(resolve, 90));
 const snaps = calls.filter(c => c.command === 'snap_nearest_edge');
 assert.equal(snaps.length, 1, 'real drag should still snap once');
 assert.deepEqual(snaps[0].args, { width:330, height:240 }, 'drag and content sizing must use identical dimensions');
 console.log('PASS: button/plain clicks stay fixed, drag alone snaps with consistent height');
 process.exit(0);
} catch (error) { console.error(error); process.exit(1); }
