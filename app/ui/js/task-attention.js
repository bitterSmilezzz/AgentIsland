import { invoke, listen } from './tauri.js';
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
let actionFeedback = null;
const feedbackKey = value => value?.error ? `error:${value.error}` : `${value?.first?.task_id}:${value?.first?.run_id}:${value?.revision}`;
let summary = null, loading = false, queued = false, notify = () => {}, visible = () => true;
export const getTaskAttention = () => summary;
function update(next) {
  if (JSON.stringify(next) === JSON.stringify(summary)) return;
  if (feedbackKey(next) !== feedbackKey(summary)) actionFeedback=null;
  summary = next; notify();
}
export async function refreshTaskAttention() {
  if (!visible()) return;
  if (loading) { queued = true; return; }
  loading = true;
  try { update(await invoke('tasks_attention_summary')); }
  catch (error) { update({ total:null, first:null, error: error?.message ?? '任务暂不可读' }); }
  finally { loading=false; if (queued) { queued=false; refreshTaskAttention(); } }
}
export async function startTaskAttention(onChange, isVisible) {
  notify=onChange; visible=isVisible;
  await Promise.all([
    listen('tasks://changed', refreshTaskAttention),
    listen('tasks://error', event => { if (visible()) update({total:null,first:null,error:event.payload?.message ?? '任务同步失败'}); }),
  ]);
  await refreshTaskAttention();
}
export function attentionDetail(item, engine) {
  if (item.detail) return item.detail;
  if (item.observed && item.target?.exactSession && engine?.session_navigation?.[item.agent_id]?.url === item.target.url) {
    const snapshot = engine.snapshots?.find(s => s.id === item.agent_id && s.level === 'attention');
    if (snapshot?.current_action) return snapshot.current_action;
  }
  return item.agent_id ? item.observed ? '在来源工具中处理' : '任务记录 · 在工具中核对' : '本地任务';
}
export function taskCoversSnapshot(value, snapshot, target) {
  const item=value?.first;
  return !!item?.observed && snapshot?.level === 'attention' && item?.agent_id === snapshot.id && item.target?.exactSession && item.target.url === target?.url;
}
export function taskCoversEvent(value, engine) {
  const item=value?.first, event=engine?.latest_event;
  if (!item?.observed || !item?.target?.exactSession || !event || event.externally_delivered || event.event_type !== 'attention' || /(?:^|\s)kind=/.test(event.detail ?? '')) return false;
  return item.agent_id === event.agent_id && engine.event_navigation?.[event.id]?.url === item.target.url;
}
export function taskAttentionHtml(value, engine) {
  if (!value) return '';
  if (value.error) return `<div class="task-attention task-attention-error"><button type="button" data-task-summary-list><strong>任务暂不可读</strong><small>${esc(value.error)}</small></button><p class="task-attention-feedback" role="status" hidden></p></div>`;
  const item=value.first;
  if (!item || !value.total) return '';
  const detail=attentionDetail(item,engine);
  const feedback=actionFeedback?.key === feedbackKey(value) ? actionFeedback.message : null;
  return `<section class="task-attention${item.failure ? ' is-failure' : ''}" data-attention-task="${esc(item.task_id)}" data-attention-run="${esc(item.run_id)}" data-attention-revision="${value.revision}" aria-label="${esc(item.label)} · 共 ${value.total} 个任务需处理">
    <div class="task-attention-head"><strong>${esc(item.label)}</strong><span title="${value.human_count} 个等待处理，${value.failed_count} 个执行失败">${value.total} 个需处理</span></div>
    <div class="task-attention-row"><button type="button" class="task-attention-title" data-task-summary-detail aria-label="查看任务：${esc(item.title)}"><strong>${esc(item.title)}</strong><small title="${esc(detail)}">${esc(detail)}</small></button>${item.target ? `<button type="button" class="task-attention-open" data-task-summary-source title="${esc(item.target.hint)}">${esc(item.target.label)}</button>` : ''}</div>
    <p class="task-attention-feedback" role="status"${feedback ? '' : ' hidden'}>${esc(feedback ?? '')}</p>
  </section>`;
}
export function bindTaskAttention(root) {
  root.querySelectorAll('[data-task-summary-detail], [data-task-summary-list], [data-task-summary-source]').forEach(button => {
    button.onclick=async event => {
      event.stopPropagation();
      const value=summary, item=value?.first;
      const container=button.closest('.task-attention');
      const feedback=container.querySelector('[role=status]');
      const general=button.hasAttribute('data-task-summary-list');
      button.disabled=true;
      try {
        if (!general && (!item || container.dataset.attentionTask !== item.task_id || container.dataset.attentionRun !== item.run_id || container.dataset.attentionRevision !== String(value.revision))) throw new Error('事项已更新，请重新点击');
        if (button.hasAttribute('data-task-summary-source')) {
          if (!item) throw new Error('事项已更新，请重新点击');
          const target=await invoke('task_open_source', {id:item.task_id,expectedRevision:value.revision});
          actionFeedback=null; feedback.hidden=true; feedback.textContent=''; button.title=target.hint;
        } else {
          await invoke('task_show_workbench', { id:general ? null : item.task_id, runId:general ? null : item.run_id, expectedRevision:general ? null : value.revision });
        }
      } catch (error) {
        feedback.hidden=false; feedback.textContent=error?.message ?? String(error);
        actionFeedback={key:feedbackKey(value),message:feedback.textContent};
        refreshTaskAttention();
      } finally { button.disabled=false; }
    };
  });
}
