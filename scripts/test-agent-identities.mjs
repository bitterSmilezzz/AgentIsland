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


const { readFileSync, existsSync, mkdirSync, writeFileSync } = await import('node:fs');
const { agentIcon, agentIdentities } = await import('../app/ui/js/agent-icons.js');
const registry = readFileSync(new URL('../app/src-tauri/src/registry.rs', import.meta.url), 'utf8');
const ids = [...registry.matchAll(/id: "([a-z-]+)"\.into\(\)/g)].map(match => match[1]);
assert.deepEqual(Object.keys(agentIdentities).sort(), ids.sort(), 'all built-in registry identities must be covered');
for (const [, mark] of Object.values(agentIdentities)) {
  if (/^[a-z]+$/.test(mark)) {
    const path = new URL(`../app/ui/assets/agents/${mark}.svg`, import.meta.url);
    assert.ok(existsSync(path), `missing local mark ${mark}`);
    const svg = readFileSync(path, 'utf8');
    assert.ok(svg.includes('<svg'));
    assert.ok(!/<script|<foreignObject|(?:href|src)=|<image|<use/i.test(svg), 'marks must be static, self-contained vectors');
  }
}
assert.ok(agentIcon({ id: 'copilot' }).includes('>IM<'), 'ima.copilot must not be confused with GitHub Copilot');
assert.ok(agentIcon({ id: 'custom', name: 'My Agent' }).includes('>MA<'));
assert.ok(agentIcon({ id: 'constructor', name: 'Custom' }).includes('>CU<'), 'inherited object properties are not catalog entries');
assert.ok(!agentIcon({ id: '\"><script>', name: '<script>' }).includes('<script>'), 'custom identifiers and initials must be escaped');
const root = mk();
const sidebarBody = mk();
root.querySelector = selector => selector === '.sb-body' ? sidebarBody : selector === '#sliver' ? mk() : null;
document.getElementById = () => root;
window.__TAURI__ = { core: { invoke: async command => command === 'get_settings' ? { appearance:'light', dock_edge:'top' } : command === 'get_boot_args' ? {} : command === 'drain_navigation' ? [] : null }, event: { listen: async () => () => {} } };
const { setState } = await import('../app/ui/js/main.js');
await new Promise(resolve => setTimeout(resolve, 30));
const snapshots = ids.map((id, i) => ({ id, name:agentIdentities[id][0], level:i % 2 ? 'attention' : 'working',
  level_label:i % 2 ? '等待确认' : '工作中', process_running:true, current_action:'示例任务',
  token_usage:{ tokens24h:12000 }, cpu_percent:12, memory_text:'120 MB', provenance_suffix:'' }));
setState({ engine: { snapshots, grand_total:{ tokens24h:312000, tokens_total:312000 }, any_working:true },
  expanded:true, route:'list', workbenchPage:'overview', settings:{ dock_edge:'top', disabled_agents:[] } });
const views = await import('../app/ui/js/views.js');
const rendered = {};
for (const [name, render] of [['island', views.renderCard], ['sidebar', views.renderSidebar], ['workbench', views.renderWorkbench]]) {
  render();
  rendered[name] = root.innerHTML;
  for (const id of ids) assert.ok(root.innerHTML.includes(`data-agent-icon="${id}"`), `${name} must show ${id} identity`);
}
const management = views.pageAgents();
for (const id of ids) {
  assert.ok(management.includes(`data-agent-icon="${id}"`), `management must show ${id} identity`);
  assert.ok(views.pageAgentDetail({ snapshots }, id).includes(`data-agent-icon="${id}"`), `detail must show ${id} identity`);
}
setState({ engine:{ snapshots:[] }, settings:{ disabled_agents:['codex'] } });
assert.ok(views.pageAgents().includes('data-agent-icon="codex"'), 'disabled agents absent from the sample must retain identity');
if (process.argv[2]) {
  mkdirSync(process.argv[2], { recursive:true });
  for (const [name, html] of Object.entries(rendered)) writeFileSync(`${process.argv[2]}/${name}.html`, html);
}
console.log('PASS: registry coverage, local vectors, safe custom initials and all five UI identity locations');
process.exit(0);
