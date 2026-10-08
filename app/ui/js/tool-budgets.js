import { invoke } from './tauri.js';
import { createToolBudgetController } from './tool-budget-state.js';
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;' }[c]));
const number = value => Number(value).toLocaleString('zh-CN');
const labels = { unset:'未设置', unavailable:'暂不可用', normal:'预算内', warning:'接近预算', exceeded:'预算超额' };
const source = row => row.source === 'disabled' ? '工具监控已关闭' : row.used == null ? '暂未取得本机用量' : `本机记录 · ${number(row.used)} tokens`;

export function toolBudgetsHtml() {
  return `<details class="tool-budgets" data-tool-budgets><summary>工具预算<span data-budget-count>本机 24h</span></summary>
    <div class="tool-budget-body"><p class="tool-budget-note">滚动 24h 净消耗 · 80% 预警 · 本地提醒，不限制请求</p>
    <input type="checkbox" hidden data-budget-lease aria-hidden="true" tabindex="-1">
    <form data-budget-form><label>工具<select data-budget-agent data-draft-ignore aria-label="预算工具" disabled></select></label>
      <label>Token 上限<input data-budget-value data-draft-ignore type="text" inputmode="numeric" autocomplete="off" spellcheck="false" aria-describedby="tool-budget-help" disabled></label>
      <div class="tool-budget-actions"><button type="submit" class="mini-btn" data-budget-save disabled>保存</button><button type="button" class="mini-btn" data-budget-cancel disabled>取消</button></div></form>
    <p id="tool-budget-help" class="tool-budget-note">0 为不设置，上限 1,000,000,000。</p>
    <div class="tool-budget-heading"><span data-budget-source>展开后读取本机快照</span><button type="button" class="mini-btn" data-budget-refresh>刷新</button></div>
    <p class="tool-budget-feedback" role="status" aria-live="polite" data-budget-status></p>
    <ul class="tool-budget-list" data-budget-list aria-label="已设置的工具预算"></ul></div></details>`;
}

export function bindToolBudgets(root = document) {
  const panel = root.querySelector('[data-tool-budgets]');
  if (!panel || panel.dataset.bound) return;
  panel.dataset.bound = 'true';
  const find = key => panel.querySelector(`[data-budget-${key}]`);
  const select = find('agent'), input = find('value');
  let options = '';
  const controller = createToolBudgetController(
    () => invoke('tool_budget_list'),
    (agentId, budget, expectedBudget) => invoke('tool_budget_set', { agentId, budget, expectedBudget }),
    state => {
      const rows = state.report?.rows ?? [];
      const nextOptions = rows.map(row => `<option value="${esc(row.agentId)}">${esc(row.name)}</option>`).join('');
      if (options !== nextOptions) { select.innerHTML = nextOptions; options = nextOptions; }
      select.value = state.selected;
      if (input.value !== state.value) input.value = state.value;
      select.disabled = input.disabled = state.busy || !state.row;
      input.setAttribute('aria-invalid', String(state.dirty && !state.valid));
      find('save').disabled = !state.canSave;
      find('cancel').disabled = state.busy || !state.dirty;
      find('refresh').disabled = state.busy;
      find('refresh').textContent = state.readFailed ? '重试' : '刷新';
      find('status').textContent = state.status;
      find('source').textContent = state.row ? source(state.row) : '尚未取得用量';
      panel.querySelector('[data-budget-count]').textContent = state.report ? `${rows.filter(row => row.budget > 0).length} 项 · 24h` : '本机 24h';
      panel.setAttribute('aria-busy', String(state.busy));
      const lease = find('lease'), keep = state.hasDraft || state.busy;
      if (lease.checked !== keep) { lease.checked = keep; lease.dispatchEvent(new Event('change', { bubbles: true })); }
      const configured = rows.filter(row => row.budget > 0);
      find('list').innerHTML = configured.map(row => `<li data-budget-state="${esc(row.status)}"><div><strong>${esc(row.name)}</strong><small>${esc(labels[row.status] ?? '暂不可用')}${row.used == null ? ` · ${esc(source(row))}` : ` · ${Math.floor(row.used / row.budget * 100)}%`}</small></div>
        <span class="tool-budget-amount">${row.used == null ? '—' : number(row.used)} <small>/ ${number(row.budget)}</small></span>
        <button type="button" class="mini-btn" data-budget-edit="${esc(row.agentId)}" aria-label="编辑 ${esc(row.name)} 预算"${state.busy ? ' disabled' : ''}>编辑</button></li>`).join('')
        || `<li class="tool-budget-empty">${state.report ? '未设置工具预算' : '预算尚未读取'}</li>`;
      if (state.report && !state.report.alertsEnabled) find('status').textContent += `${state.status ? ' · ' : ''}预算提醒已关闭，可在设置中开启`;
    },
  );
  panel.addEventListener('toggle', () => { if (panel.open && !controller.state.report) controller.load(); });
  find('form').addEventListener('submit', async event => {
    event.preventDefault();
    await controller.save();
    // Disabling a pending field drops WebKit focus. Restore it only if the user
    // has not moved focus elsewhere, and never focus a cached or collapsed page.
    if (panel.isConnected && panel.open && !input.disabled && document.activeElement === document.body)
      input.focus({ preventScroll: true });
  });
  select.addEventListener('change', () => controller.select(select.value));
  input.addEventListener('input', () => controller.edit(input.value));
  find('cancel').addEventListener('click', () => { controller.cancel(); input.focus({ preventScroll: true }); });
  find('refresh').addEventListener('click', () => controller.load());
  find('list').addEventListener('click', event => {
    const button = event.target.closest('[data-budget-edit]');
    if (button) { controller.select(button.dataset.budgetEdit); input.focus({ preventScroll: true }); }
  });
}
