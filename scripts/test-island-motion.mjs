import assert from 'node:assert/strict';
import { navigationActive, stageNavigation, resizeNavigation } from '../app/ui/js/island-navigation.js';

// Browser WAAPI replaces Animation.finished with a new pending promise on cancel.
// Exercise the real resize controller with that contract, rather than a resolved stub.
class Animation {
  constructor() { this.reset(); this.timer = setTimeout(() => this.resolve(), 20); }
  reset() { this.promise = new Promise((resolve, reject) => { this.resolve = resolve; this.reject = reject; }); }
  get finished() { return this.promise; }
  cancel() { clearTimeout(this.timer); this.reject(new Error('cancelled')); this.reset(); }
}
class Element {
  constructor(width, height) {
    this.width = width; this.height = height; this.style = {}; this.dataset = {}; this.children = [];
    this.classList = { add() {}, remove() {} }; this.isConnected = true;
  }
  getBoundingClientRect() { return { width: this.width, height: this.height }; }
  querySelector(selector) { return selector === '.card-inner' ? this.inner : null; }
  querySelectorAll() { return []; }
  appendChild(node) { this.children.push(node); }
  remove() { this.removed = true; }
  animate() { return new Animation(); }
}
globalThis.window = { innerWidth: 330, innerHeight: 520 };
globalThis.requestAnimationFrame = callback => setTimeout(callback, 0);
globalThis.getComputedStyle = () => ({ opacity: '1', transform: 'none' });
const card = new Element(330, 306);
card.inner = new Element(328, 138);
const copy = new Element(298, 304);
const calls = [];
let settled = 0;
const invoke = async (_, size) => calls.push(size);
stageNavigation(card, { copy, from: { width: 300, height: 306 }, route: 'tokenAnalytics', previous: 'list' });
const first = resizeNavigation(card, invoke, () => true, () => settled++);
await new Promise(resolve => setTimeout(resolve, 5));
card.inner.height = 518; // A report arrives while the first geometry animation runs.
const final = resizeNavigation(card, invoke, () => true, () => settled++);
await Promise.race([final, new Promise((_, reject) => setTimeout(() => reject(new Error('retarget never settled after cancelling the previous animation')), 300))]);
await first;
assert.equal(navigationActive(card), false);
assert.equal(settled, 1);
assert.equal(copy.removed, true);
assert.deepEqual(calls.at(-1), { width: 330, height: 520 });
assert(!calls.some(size => size.height === 140), 'an obsolete target must not contract the native envelope');
assert.equal(calls.length, 3, 'reserve, retarget reservation, final release; never one call per visual frame');
console.log('PASS: asynchronous report retarget settles after real WAAPI cancellation semantics');

// Sampling can remove a 16px action line immediately after an Agent return.
// Animate only the surface; do not replay page content or steal keyboard focus.
const sampled = new Element(300, 328);
sampled.inner = new Element(298, 326);
sampled.inner.querySelector = selector => {
  if (selector === '.page') throw new Error('sampling must not replay page content');
  if (selector === '[data-back]') throw new Error('sampling must not move keyboard focus');
  return null;
};
const sampledCopy = new Element(298, 342);
stageNavigation(sampled, { copy: sampledCopy, from: { width: 300, height: 344 }, route: 'list', previous: 'list', layoutOnly: true });
await resizeNavigation(sampled, async (_, size) => Object.assign(window, { innerWidth: size.width, innerHeight: size.height }), () => true, () => {});
assert.equal(navigationActive(sampled), false);
assert.equal(sampledCopy.removed, true);
console.log('PASS: post-return sampling animates height without replaying content or moving focus');
