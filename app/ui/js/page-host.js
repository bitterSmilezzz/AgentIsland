// Retain page DOM in this WebView only: no form values enter storage or IPC.
export class PageCache {
  constructor() { this.pages = new Map(); }
  remember(key, node) { if (key && node) this.pages.set(key, node); }
  take(key) { const node = this.pages.get(key); this.pages.delete(key); return node; }
  get retainedControls() {
    return [...this.pages.values()].flatMap(node => [...node.querySelectorAll('input, select, textarea')]);
  }
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
