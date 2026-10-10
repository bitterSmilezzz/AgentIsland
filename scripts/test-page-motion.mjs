import assert from 'node:assert/strict';

globalThis.location = { search: '?shell=workbench' };
globalThis.document = { documentElement: { classList: { contains: name => name === 'shell-workbench' } } };
let reduced = false;
globalThis.matchMedia = () => ({ matches: reduced });
globalThis.getComputedStyle = node => ({ opacity: node.opacity ?? '1' });
const { rememberPage, pageMotion } = await import('../app/ui/js/page-motion.js');

class Node {
  constructor(parent, name = 'page') {
    Object.assign(this, { parentElement: parent, name, opacity: '1', scrollTop: 0,
      scrollLeft: 0, style: {}, dataset: {}, children: [], animations: [],
      classList: { add() {}, remove() {} } });
  }
  getBoundingClientRect() { return { left: 0, top: 60, width: 800, height: 600 }; }
  querySelectorAll(selector) { return selector === '[id]' ? [] : this.children; }
  setAttribute() {} removeAttribute() {}
  cloneNode() { const copy = new Node(this.parentElement, this.name); copy.children = this.children.map(n => n.cloneNode()); return copy; }
  appendChild(node) { this.children.push(node); node.parentElement = this; }
  remove() {
    const parent = this.parentElement;
    if (parent.copies) parent.copies = parent.copies.filter(n => n !== this);
    else parent.children = parent.children.filter(n => n !== this);
  }
  animate(frames, options) {
    let finish, reject;
    const finished = new Promise((resolve, no) => { finish = resolve; reject = no; });
    const animation = { frames, options, finished, finish, cancel() { reject(new Error('cancelled')); } };
    this.animations.push(animation);
    return animation;
  }
}
document.createElement = () => new Node(null, 'cover');
const descendants = nodes => nodes.flatMap(node => [node, ...descendants(node.children)]);
const copies = root => root.querySelectorAll('[data-page-outgoing]');
function fixture() {
  const host = { copies: [], classList: { add() {}, remove() {} },
    getBoundingClientRect: () => ({ left: 0, top: 60 }), appendChild(node) { this.copies.push(node); node.parentElement = this; } };
  const root = { dataset: { motionRoute: 'settings' }, host,
    querySelector() { return this.current; },
    querySelectorAll(selector) {
      const key = selector === '[data-page-outgoing]' ? 'pageOutgoing' : selector === '[data-page-cover]' ? 'pageCover' : null;
      return key ? descendants(host.copies).filter(n => n.dataset[key] != null) : [host];
    } };
  root.current = new Node(host, 'settings');
  return root;
}
function navigate(root, key) {
  rememberPage(root, key, '.wb-content');
  root.current = new Node(root.host, key);
  pageMotion(root, key, '.wb-content');
  return root.current;
}

const root = fixture();
root.current.scrollTop = 180;
const nested = new Node(root.host, 'nested'); nested.scrollLeft = 40;
root.current.children.push(nested);
const incoming = navigate(root, 'tokenAnalytics');
const outgoing = copies(root)[0];
const entry = incoming.animations[0], exit = outgoing.animations[0];
assert.equal(entry.frames[0].opacity, .4, 'handoff must keep content readable instead of flashing blank');
assert.ok(entry.options.delay >= exit.options.duration, 'two readable pages must not overlap');
assert.equal(outgoing.parentElement.style.background, 'var(--surface)', 'solid cover hides the destination during exit');
assert.equal(exit.frames.at(-1).opacity, .4, 'outgoing content must never fully disappear');
assert.equal(outgoing.scrollTop, 180, 'outgoing snapshot must retain its scroll position');
assert.equal(outgoing.children[0].scrollLeft, 40, 'nested viewport must not jump in the snapshot');
assert.equal(outgoing.inert, true);

// Reverse before the new page becomes visible: continue the visible source.
incoming.opacity = '0'; outgoing.opacity = '.4';
const returned = navigate(root, 'settings');
assert.equal(root.host.copies.length, 0, 'return must not keep a second copy of the same page');
assert.equal(returned.animations[0].frames[0].opacity, .4, 'return resumes the visible source opacity');
assert.equal(returned.animations[0].options.delay, 0, 'reverse navigation has no second exit wait');
await Promise.resolve(); await Promise.resolve();
assert.equal(returned.animations.length, 1, 'cancelled completion cannot start another entry');

// A third target during the exit preserves what is actually visible.
const thirdRoot = fixture(); const waiting = navigate(thirdRoot, 'tokenAnalytics');
waiting.opacity = '.4'; copies(thirdRoot)[0].opacity = '.6';
navigate(thirdRoot, 'tasks');
assert.equal(copies(thirdRoot)[0].name, 'settings');
assert.equal(copies(thirdRoot)[0].animations[0].frames[0].opacity, .6);
assert.equal(thirdRoot.host.copies.length, 1);

// Switching from whole-page navigation to a task classification must not
// transplant the old whole-page cover into a smaller nested content panel.
const nestedRoot = fixture(); navigate(nestedRoot, 'tasks');
rememberPage(nestedRoot, 'todo', '[data-task-view-panel]');
nestedRoot.current = new Node(nestedRoot.host, 'todo');
pageMotion(nestedRoot, 'todo', '[data-task-view-panel]');
assert.equal(copies(nestedRoot)[0].name, 'tasks');
assert.equal(copies(nestedRoot)[0].dataset.pageSurface, '[data-task-view-panel]');
const growing = fixture(); rememberPage(growing, 'tasks', '.wb-content');
growing.current = new Node(growing.host, 'tasks');
growing.current.getBoundingClientRect = () => ({ left: 0, top: 60, width: 800, height: 900 });
pageMotion(growing, 'tasks', '.wb-content');
assert.equal(growing.host.copies[0].style.height, '900px', 'cover must hide the entire taller destination');
assert.equal(copies(growing)[0].style.height, '600px', 'snapshot keeps its original geometry');

// Current-page updates and reduced motion never create navigation layers.
const same = fixture(); rememberPage(same, 'settings', '.wb-content'); pageMotion(same, 'settings', '.wb-content');
assert.equal(same.current.animations.length, 0);
reduced = true; const direct = fixture(); navigate(direct, 'tasks');
assert.equal(direct.current.animations.length, 0); assert.equal(direct.host.copies.length, 0);
reduced = false; const interrupted = fixture(); navigate(interrupted, 'tokenAnalytics');
reduced = true; const settled = navigate(interrupted, 'settings');
assert.equal(settled.animations.length, 0); assert.equal(interrupted.host.copies.length, 0);
console.log('PASS: workbench text handoff, early reverse, third target, scroll snapshots and reduced motion');
