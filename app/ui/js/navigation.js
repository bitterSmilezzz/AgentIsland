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
