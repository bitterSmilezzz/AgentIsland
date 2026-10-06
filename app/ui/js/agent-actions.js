// Task state and resource health answer different questions.
export function healthReason(snapshot) {
  const issues = snapshot.health?.issues ?? [];
  if (snapshot.is_hung === true) return { label: '疑似卡死', detail: issues.join('；') || '持续高负荷，请检查任务是否仍有进展' };
  const memory = issues.find(s => s.includes('内存连续增长'));
  if (memory) return { label: '内存增长', detail: memory };
  const cpu = issues.find(s => s.includes('CPU'));
  if (cpu) return { label: 'CPU 偏高', detail: cpu };
  return issues.length ? { label: '资源提醒', detail: issues.join('；') } : null;
}
export function agentNextStep(snapshot, target) {
  const health = healthReason(snapshot);
  let kind, detail, action;
  if (snapshot.level === 'attention') {
    kind = '待确认'; detail = snapshot.current_action || '会话正在等待你确认'; action = '去确认';
  } else if (snapshot.level === 'completed') {
    kind = '已完成'; detail = '查看本轮结果'; action = '查看结果';
  } else if (health) {
    kind = health.label; detail = health.detail; action = '检查会话';
  } else return null;
  return { kind, detail, action: target ? (target.exactSession ? action : '打开工具') : null,
    hint: target?.hint ?? '此智能体尚未接入工具跳转' };
}
