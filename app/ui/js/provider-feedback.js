import { pageOwnsFocus } from './page-host.js';

// A delayed write may reveal its receipt only while the user stays with that
// operation. Refreshing a form after they start typing would also lose a draft.
export function beginProviderFeedback(root) {
  const doc = root.ownerDocument;
  let owned = pageOwnsFocus(root, doc.activeElement);
  const release = () => { owned = false; };
  const changedFocus = event => {
    if (event.target !== doc.body) release();
  };
  const events = [['focusin', changedFocus], ['pointerdown', release],
    ['keydown', release], ['wheel', release]];
  for (const [name, listener] of events) doc.addEventListener(name, listener, { capture: true });
  doc.defaultView.addEventListener('blur', release);
  const current = () => owned && root.isConnected &&
    !root.closest('[hidden], [inert], [data-page-outgoing]') &&
    root.getClientRects().length > 0;
  return {
    current,
    reveal(node) {
      if (!current()) return;
      node.focus({ preventScroll: true });
      node.scrollIntoView({ block: 'nearest', behavior: 'auto' });
    },
    dispose() {
      for (const [name, listener] of events) doc.removeEventListener(name, listener, { capture: true });
      doc.defaultView.removeEventListener('blur', release);
    },
  };
}

export function showProviderFeedback(root, text, operation) {
  let node = root.querySelector('[data-toast]');
  if (!node) {
    node = root.ownerDocument.createElement('div');
    node.className = 'sb-toast';
    node.setAttribute('data-toast', '');
    node.setAttribute('role', 'status');
    node.setAttribute('aria-live', 'polite');
    node.setAttribute('aria-atomic', 'true');
    node.tabIndex = -1;
    const summary = root.querySelector('[data-status]');
    if (summary) summary.after(node);
    else root.prepend(node);
  }
  node.hidden = false;
  node.textContent = text;
  operation?.reveal(node);
}
