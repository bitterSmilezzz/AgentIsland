// Retain page DOM in this WebView only: no form values enter storage or IPC.
export class PageCache {
  constructor() { this.pages = new Map(); }
  remember(key, node) { if (key && node) this.pages.set(key, node); }
  take(key) { const node = this.pages.get(key); this.pages.delete(key); return node; }
  get retainedControls() {
    return [...this.pages.values()].flatMap(node => [...node.querySelectorAll('input, select, textarea')]);
  }
}

// A preview container can receive focus from the toolbar. Enter its controls
// from the matching end instead of treating that container as the first item.
export function previewFocusTarget(controls, active, backwards = false) {
  if (!controls.length) return null;
  const index = controls.indexOf(active);
  return controls[index < 0 ? (backwards ? controls.length - 1 : 0)
    : (index + (backwards ? -1 : 1) + controls.length) % controls.length];
}

// Repeated reads on the same container must not overwrite a newer response.
const requests = new WeakMap();
export function pageRequest(root) {
  const token = {};
  requests.set(root, token);
  const current = () => root.isConnected !== false && requests.get(root) === token;
  current.ownsRequest = () => requests.get(root) === token;
  return current;
}

// Reveal only the selected navigation item. Do not scroll the page or reset a
// user's menu position when the item already fits inside the menu viewport.
export function revealNavigationItem(menu) {
  const item = menu?.querySelector('[aria-current="page"]');
  if (!item || menu.clientHeight <= 0) return;
  const viewport = menu.getBoundingClientRect(), rect = item.getBoundingClientRect();
  const inset = 6;
  const delta = rect.top < viewport.top + inset ? rect.top - viewport.top - inset
    : rect.bottom > viewport.bottom - inset ? rect.bottom - viewport.bottom + inset : 0;
  if (delta) menu.scrollTop = Math.max(0, Math.min(menu.scrollHeight - menu.clientHeight, menu.scrollTop + delta));
}
