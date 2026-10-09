import assert from 'node:assert/strict';
import { beginProviderFeedback, showProviderFeedback } from '../app/ui/js/provider-feedback.js';

const previousDocument = globalThis.document;
const doc = new EventTarget();
doc.body = {}; doc.activeElement = {}; doc.defaultView = new EventTarget();
globalThis.document = doc;
let hidden = false, visible = true, focused = 0, scrolled = 0;
const root = { ownerDocument: doc, isConnected: true,
  contains: node => node === doc.activeElement,
  closest: () => hidden ? {} : null,
  getClientRects: () => visible ? [{}] : [],
};
const node = { focus: () => focused++, scrollIntoView: () => scrolled++ };
try {
  let operation = beginProviderFeedback(root);
  operation.reveal(node);
  assert.equal(focused, 1); assert.equal(scrolled, 1);
  operation.dispose();
  for (const event of ['focusin', 'pointerdown', 'keydown', 'wheel']) {
    operation = beginProviderFeedback(root);
    doc.dispatchEvent(new Event(event));
    assert.equal(operation.current(), false, `${event} relinquishes delayed refresh and reveal`);
    operation.reveal(node); operation.dispose();
  }
  operation = beginProviderFeedback(root);
  doc.defaultView.dispatchEvent(new Event('blur'));
  assert.equal(operation.current(), false); operation.dispose();
  assert.equal(focused, 1); assert.equal(scrolled, 1);
  for (const state of ['disconnected', 'hidden', 'no-layout']) {
    operation = beginProviderFeedback(root);
    root.isConnected = state !== 'disconnected'; hidden = state === 'hidden'; visible = state !== 'no-layout';
    operation.reveal(node); operation.dispose();
    root.isConnected = true; hidden = false; visible = true;
  }
  assert.equal(focused, 1); assert.equal(scrolled, 1);
  operation = beginProviderFeedback(root); operation.dispose();
  doc.dispatchEvent(new Event('pointerdown'));
  assert.equal(operation.current(), true, 'disposed listeners no longer react');

  const elements = [];
  doc.createElement = () => ({ setAttribute() {}, hidden: true });
  const summary = { after: node => elements.push(node) };
  root.querySelector = selector => selector === '[data-status]' ? summary : elements[0];
  root.prepend = node => elements.unshift(node);
  const literal = '<img src=x onerror=alert(1)> backup';
  showProviderFeedback(root, literal);
  assert.equal(elements[0].textContent, literal); assert.equal(elements[0].hidden, false);
  showProviderFeedback(root, '下一次结果');
  assert.equal(elements.length, 1); assert.equal(elements[0].textContent, '下一次结果');
  elements.length = 0;
  root.querySelector = () => null;
  showProviderFeedback(root, '写入成功，状态刷新失败');
  assert.equal(elements[0].textContent, '写入成功，状态刷新失败');
  console.log('PASS: provider receipts stay inline and literal; late refresh/reveal yields to user interaction, hidden pages and window blur');
} finally {
  if (previousDocument === undefined) delete globalThis.document;
  else globalThis.document = previousDocument;
}
