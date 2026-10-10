import { invoke, listen } from './tauri.js';

// One directory for navigation and recreated WebViews; saved values are page
// keys only. Keep the legacy report entry as a launch target, not a second page.
export const workbenchPages = [
  ['overview', '概览', 'square'], ['sessions', '会话', 'terminal'], ['tokenAnalytics', '用量分析', 'chart'],
  ['todo', '待办事项', 'check'], ['provider', '模型与连接', 'sliders'],
  ['tasks', '任务', 'check'], ['workspaces', '工作空间', 'square'], ['windows', '窗口排列', 'square'], ['agents', '智能体管理', 'terminal'],
  ['remote', '远程通知', 'bell'], ['settings', '设置', 'gear'],
];
export function initialWorkbenchPage(launch, saved) {
  const page = launch || saved;
  return page === 'report' || workbenchPages.some(([key]) => key === page) ? page : 'overview';
}

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

export function taskIdFromIntent(intent) {
  if (!intent.startsWith('Task(') || !intent.endsWith(')')) return null;
  try {
    const id = JSON.parse(intent.slice(5,-1));
    return typeof id === 'string' && /^[a-f0-9]{8}-(?:[a-f0-9]{4}-){3}[a-f0-9]{12}$/i.test(id) ? id : null;
  } catch { return null; }
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
