// Tauri API 薄封装（withGlobalTauri 全局）
export async function invoke(cmd, args = {}) {
  const t = window.__TAURI__;
  if (!t?.core?.invoke) throw new Error('Tauri API 未就绪');
  return t.core.invoke(cmd, args);
}

export async function listen(event, handler) {
  const t = window.__TAURI__;
  if (!t?.event?.listen) return () => {};
  return t.event.listen(event, handler);
}

export function getCurrentWindow() {
  return window.__TAURI__.window.getCurrentWindow();
}
