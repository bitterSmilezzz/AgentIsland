import { invoke, listen } from './tauri.js';

// Rust 的 Debug 意图中 Agent(String) 使用带引号、转义的字符串。
// 三种窗口共用解码，避免把引号带进路由中的 agent id。
export function agentIdFromIntent(intent) {
  if (!intent.startsWith('Agent(') || !intent.endsWith(')')) return null;
  try {
    const id = JSON.parse(intent.slice(6, -1));
    return typeof id === 'string' && id.length > 0 ? id : null;
  } catch {
    return null;
  }
}

// 先装订阅，再逐批消费启动缓存；空批次确认后 Rust 才发实时事件。
export async function subscribeNavigation(handler) {
  const process = async event => {
    try { await handler(event); }
    catch (error) {
      await invoke('log_from_ui', { message: `深链导航失败：${error?.message ?? error}` }).catch(() => {});
    }
  };
  let chain = Promise.resolve();
  await listen('deeplink://navigate', event => {
    chain = chain.then(() => process(event));
    return chain;
  });
  while (true) {
    const pending = await invoke('drain_navigation');
    if (!pending.length) break;
    for (const action of pending) await process({ payload: { action } });
  }
}
