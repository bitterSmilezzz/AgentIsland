// Tauri API 薄封装（withGlobalTauri 全局）

/**
 * 等 Tauri 的全局 API 就绪。
 *
 * **为什么必须等**：`boot()` 在模块求值那一刻就跑，而 `window.__TAURI__`
 * 是 Tauri 之后才注入的。抢在它之前调用：
 * · 每个 `invoke` 都抛「Tauri API 未就绪」，而调用点全带 `.catch()` ⇒ **全被吞掉**；
 * · 更要命的是 `listen()` 此时**返回一个空函数**，于是 `engine://tick`
 *   永远没有订阅上——即使 API 后来就绪，界面也不会再刷新一次。
 *
 * 症状与「页面没加载」几乎一样（空白 + `[webview]` 零行），所以必须显式等，
 * 并把等没等到**写进日志**——否则又是一次「没有日志 ⇒ 猜」。
 */
export async function waitForTauri(timeoutMs = 5000) {
  const started = Date.now();
  while (!window.__TAURI__?.core?.invoke) {
    if (Date.now() - started > timeoutMs) {
      return { ready: false, waitedMs: Date.now() - started };
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  return { ready: true, waitedMs: Date.now() - started };
}
let motionProbeReportDelay = 0;
// Explicit native probe only: the backend refuses this capability on ordinary startup.
export async function setMotionProbeReportDelay(delay) {
  if (![0, 650].includes(delay)) throw new Error('Invalid motion probe delay');
  await invoke('motion_probe_frame');
  motionProbeReportDelay = delay;
}
export async function invoke(cmd, args = {}) {
  const t = window.__TAURI__;
  if (!t?.core?.invoke) throw new Error('Tauri API 未就绪');
  const result = t.core.invoke(cmd, args);
  if (cmd === 'get_report' && motionProbeReportDelay) {
    // Attach the rejection handler immediately, even while delivery is delayed.
    const [response] = await Promise.all([result, new Promise(resolve => setTimeout(resolve, motionProbeReportDelay))]);
    return response;
  }
  return result;
}

export async function listen(event, handler) {
  const t = window.__TAURI__;
  if (!t?.event?.listen) return () => {};
  return t.event.listen(event, handler);
}

export function getCurrentWindow() {
  return window.__TAURI__.window.getCurrentWindow();
}
