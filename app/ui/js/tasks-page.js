import { watchArtifactLifetime } from './artifact-lifetime.js';
import { invoke, listen } from './tauri.js';
import { pageRequest } from './page-host.js';
import { retainTaskFocus } from './task-focus.js';
import { chiefResourcesPage, bindChiefResources } from './chief-resources-page.js';
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const statuses = { queued: '未开始', running: '执行中', waiting: '等待处理', ready: '结果就绪', failed: '执行失败', cancelled: '已取消', accepted: '来源已验收' };
const kinds = { answer: '待回答', plan_approval: '确认方案', result_review: '验收结果', confirmation: '需要确认' };
const boardColumns = { attention: '需处理', running: '执行中', queued: '未开始', finished: '已结束' };
const statusChoices = {
  attention: [
    ['waiting:answer', 'waiting', 'answer', '等待处理 · 待回答'],
    ['waiting:plan_approval', 'waiting', 'plan_approval', '等待处理 · 确认方案'],
    ['ready:result_review', 'ready', 'result_review', '等待处理 · 验收结果'],
    ['waiting:confirmation', 'waiting', 'confirmation', '等待处理 · 需要确认'],
    ['failed', 'failed', null, '执行失败'],
  ],
  running: [['running', 'running', null, '执行中']],
  queued: [['queued', 'queued', null, '未开始']],
  finished: [['ready', 'ready', null, '结果就绪'], ['cancelled', 'cancelled', null, '已取消']],
};
export function taskGateNotice(attention) {
  if (!attention.observed) return '本地记录，处理后标记即可。';
  return attention.kind === 'plan_approval' ? '来源未提供方案正文，请在工具中查看。' : '请在来源工具中处理，再更新本地记录。';
}
const draftActions = label => `<div class="task-form-actions"><button class="mini-btn" type="button" data-task-discard hidden>撤销修改</button><button class="mini-btn" type="submit">${label}</button></div><p class="task-form-feedback" data-task-form-feedback role="status" aria-live="polite" hidden></p>`;
export function runSourceAction(run) {
  if (!run.source) return null;
  return { runId: run.id, label: run.source.thread_id && run.source.agent_id === 'codex' ? '打开来源会话' : '打开来源工具' };
}
export function artifactReadAction(artifact, runId) {
  if (!artifact?.event_ref && !artifact?.local_content?.sha256) return null;
  return { artifactId: artifact.id, runId, label:artifact.event_ref ? (artifact.kind === 'answer' ? '查看问题' : artifact.kind === 'plan_approval' ? '查看方案' : '查看结果') : '查看产出' };
}
function artifactReadButton(artifact, runId) {
  const action=artifactReadAction(artifact,runId);
  return action ? `<button class="mini-btn" type="button" data-task-artifact-read="${esc(action.artifactId)}" data-artifact-run="${esc(action.runId)}">${esc(action.label)}</button>` : '';
}
export function renderTaskHistory(data, taskId) {
  const history = data.runs.filter(r => r.task_id === taskId).slice().reverse();
  return `<details class="task-history"><summary>运行记录 · ${history.length}</summary>${history.length ? history.map((r, i) => {
    const action = runSourceAction(r);
    return `<article><div class="task-history-heading"><strong>第 ${history.length - i} 次 · ${esc(statuses[r.status])}</strong>${action ? `<button type="button" class="mini-btn" data-task-run-open="${esc(action.runId)}">${esc(action.label)}</button>` : '<span class="task-history-local">未保存来源</span>'}</div><time>${esc(new Date(r.started_ms).toLocaleString())}</time>${data.artifacts.filter(a => a.run_id === r.id && !data.attentions.some(g => g.artifact_id === a.id)).map(a => `<p class="task-artifact-line"><span>产出引用${a.local_content?.sha256 ? ` · 版本 ${a.revision}` : ''} · ${esc(a.title)}</span>${artifactReadButton(a,r.id)}</p>`).join('')}${data.attentions.filter(a => a.run_id === r.id).map(a => `<p>${esc(kinds[a.kind])} · ${a.state === 'expired' ? '已过期' : a.state === 'open' ? '待处理' : a.manual_resolution ? '本地已处理' : '来源已处理'}${a.artifact_id ? ` · ${esc(data.artifacts.find(x => x.id === a.artifact_id)?.title)}` : ''}${artifactReadButton(data.artifacts.find(x => x.id === a.artifact_id),r.id)}</p>`).join('')}</article>`;
  }).join('') : '<p>尚无运行记录</p>'}</details>`;
}
function localArtifactChains(data, taskId) {
  const runIds = new Set(data.runs.filter(run => run.task_id === taskId).map(run => run.id));
  const groups = new Map();
  for (const artifact of data.artifacts) {
    if (!runIds.has(artifact.run_id) || artifact.event_ref || !artifact.local_content?.sha256) continue;
    const logicalId = artifact.logical_id ?? artifact.id;
    const chain = groups.get(logicalId) ?? [];
    chain.push(artifact);
    groups.set(logicalId, chain);
  }
  return [...groups.entries()].map(([logicalId, versions]) => ({
    logicalId,
    versions: versions.slice().sort((a, b) => a.revision - b.revision),
  }));
}
function checklistSummary(checklist) {
  const items = Array.isArray(checklist?.items) ? checklist.items : [];
  if (!items.length) return '未添加验证项 · 尚未通过';
  const failed = items.filter(item => item.status === 'failed').length;
  const pending = items.filter(item => item.status === 'pending').length;
  if (failed) return `有 ${failed} 项未通过${pending ? ` · ${pending} 项待验` : ''}`;
  if (pending) return `${pending} 项待验`;
  return '全部通过';
}
function renderArtifactChecklist(version, editable) {
  const items = Array.isArray(version.checklist?.items) ? version.checklist.items : [];
  const statusNames = { pending: '待验', passed: '通过', failed: '未通过' };
  return `<details class="task-checklist" data-task-checklist data-artifact-id="${esc(version.id)}">
    <summary><span>验证清单</span><span class="task-checklist-summary">${esc(checklistSummary(version.checklist))}</span></summary>
    ${items.length ? `<ul>${items.map(item => `<li class="task-checklist-item"><span class="task-checklist-title">${esc(item.title)}</span>${editable ? `<form data-task-checklist-mark data-artifact-id="${esc(version.id)}" data-checklist-item-id="${esc(item.id)}"><label><span class="sr-only">${esc(item.title)}的验证结果</span><select name="status">${Object.entries(statusNames).map(([value, label]) => `<option value="${value}"${item.status === value ? ' selected' : ''}>${label}</option>`).join('')}</select></label>${draftActions('保存状态')}</form>` : `<span class="task-checklist-status">${esc(statusNames[item.status] ?? '状态无效')}</span>`}</li>`).join('')}</ul>` : '<p class="task-checklist-empty">还没有验证项；空清单不会显示为通过。</p>'}
    ${editable ? `<form class="task-checklist-add" data-task-checklist-add data-artifact-id="${esc(version.id)}"><label><span>添加验证项</span><input name="title" maxlength="500" required placeholder="写下可核对的本地验收项"></label>${draftActions('添加')}</form>` : ''}
  </details>`;
}
function renderArtifactVersions(data, task) {
  const chains = localArtifactChains(data, task.id);
  if (!chains.length) return '';
  return `<section class="task-artifact-versions" aria-label="本地产出版本"><h3>本地产出版本</h3>${chains.map(chain => {
    const latest = chain.versions.at(-1);
    const fromOptions = chain.versions.map(version => `<option value="${esc(version.id)}">版本 ${version.revision} · ${esc(version.title)}</option>`).join('');
    const toOptions = chain.versions.map((version, index) => `<option value="${esc(version.id)}"${index === chain.versions.length - 1 ? ' selected' : ''}>版本 ${version.revision} · ${esc(version.title)}</option>`).join('');
    return `<details class="task-version-chain"><summary>${esc(latest.title)} · ${chain.versions.length} 个版本</summary>
      <ol>${chain.versions.map(version => `<li class="task-version-row"><div><span>版本 ${version.revision} · ${esc(version.title)}</span><span class="task-version-actions">${artifactReadButton(version, version.run_id)}${chain.versions.length > 1 ? `<button class="mini-btn" type="button" data-task-artifact-restore="${esc(version.id)}">恢复到此版（会新增版本）</button>` : ''}</span></div>${renderArtifactChecklist(version, task.archived_ms == null)}</li>`).join('')}</ol>
      ${chain.versions.length > 1 ? `<form class="task-version-compare" data-task-version-compare><label>从<select name="fromArtifactId">${fromOptions}</select></label><label>比较到<select name="toArtifactId">${toOptions}</select></label><button class="mini-btn" type="submit">比较版本</button><p class="task-form-feedback" data-version-feedback role="status" aria-live="polite" hidden></p></form>` : ''}
      ${task.archived_ms == null ? `<form class="task-version-form" data-task-version-form data-artifact-id="${esc(latest.id)}"><label>新版本名称<input name="artifactTitle" value="${esc(latest.title)}" maxlength="500" required></label><label>新版本正文<textarea name="artifactContent" rows="4" maxlength="65536" required placeholder="最多 64 KiB；可识别的凭据内容不会保存"></textarea></label>${draftActions('记录新版本')}</form>` : ''}
    </details>`;
  }).join('')}</section>`;
}
export function taskGroup(data, task) {
  const run = data.runs.find(r => r.id === task.current_run_id);
  if (!run) return 'queued';
  if (data.attentions.some(a => a.run_id === run.id && a.state === 'open') || run.status === 'waiting' || run.status === 'failed') return 'attention';
  if (run.status === 'running') return 'running';
  return run.status === 'queued' ? 'queued' : 'finished';
}
export function pageTasks() {
  return `<section class="task-space" data-tasks-root>
    <div class="task-panes">
    <div class="task-list-pane">
    <div class="task-controls">
    <div class="task-toolbar"><label class="task-filter">项目<select data-task-filter aria-label="筛选项目"><option value="">全部项目</option></select></label><label class="task-search">搜索<input type="search" data-task-search aria-label="搜索任务（跨项目，按名称或来源）" placeholder="名称或来源…" autocomplete="off"></label><label class="task-archive-filter"><input type="checkbox" data-task-archived>已归档</label><button class="mini-btn" type="button" data-task-refresh>刷新</button></div>
    <form class="task-compose" data-task-create><label class="sr-only" for="task-new-title">任务名称</label><input id="task-new-title" name="title" placeholder="记录一个任务…" maxlength="500" required autocomplete="off"><select name="projectId" aria-label="任务所属项目"><option value="">未分组</option></select><button class="mini-btn primary" type="submit">添加任务</button></form>
    <details class="task-project-create"><summary>管理项目</summary><form data-task-project><input name="name" aria-label="新项目名称" placeholder="项目名称" maxlength="500" required autocomplete="off"><button class="mini-btn" type="submit">添加项目</button></form><section class="task-project-manager" data-task-project-manager aria-label="项目管理"><label>配置项目<select data-task-project-select aria-label="选择要配置的项目"><option value="">选择项目</option></select></label><form data-task-project-dependencies hidden></form><section class="task-project-authorization" data-task-project-authorization hidden aria-label="项目运行授权"></section><div class="task-project-delete" data-task-project-delete hidden><p data-project-task-count></p><button class="mini-btn" type="button" data-task-project-delete-button>删除项目</button></div></section></details>
    <button class="mini-btn task-chief-open" type="button" data-chief-open>团队资源配置</button>
    <section class="task-project-targets" data-task-project-targets hidden aria-label="任务拖放目标"><p>项目目标调整归属；状态目标会先让你选择具体的本地进度。拖放不会启动、停止或批准外部智能体。</p><div data-task-project-target-list role="list"></div><div class="task-status-targets" data-task-status-targets><span>记录本地进度</span><div data-task-status-target-list role="list"></div></div></section>
    </div>
    <div class="task-board" data-task-board></div>
    </div>
    <section class="task-detail" tabindex="-1" data-task-detail hidden aria-label="任务详情"></section>
    <aside class="task-detail-empty"><strong>选择任务</strong><p>查看来源、记录进度与运行历史。</p></aside>
    </div>
    <div class="task-footer"><p class="task-feedback" data-task-feedback role="status" aria-live="polite">正在读取任务</p><p class="task-note">仅整理本地记录，不会启动任务或批准操作。</p></div>
    ${chiefResourcesPage()}
    <dialog class="task-status-dialog" data-task-status-dialog aria-labelledby="task-status-dialog-title"><form data-task-status-form><h2 id="task-status-dialog-title" data-task-status-title>记录本地进度</h2><p class="task-note">这只会更新本机任务记录，不会启动、停止或批准外部智能体；“来源已验收”不能通过拖放设置。</p><label>具体进度<select name="progress" data-task-status-select required></select></label><p class="task-form-feedback" data-task-status-feedback role="status" aria-live="polite" hidden></p><div class="task-form-actions"><button class="mini-btn" type="button" data-task-status-cancel>取消</button><button class="mini-btn" type="submit">确认记录</button></div></form></dialog>
  </section>`;
}
let requestedTask = null;
let requestedProject = null;
export function selectTaskProject(id) {requestedProject=id;requestedTask=null;document.querySelector('[data-tasks-root]')?.selectTask?.();}
export function selectTask(id) {
  requestedTask = id;
  const root = document.querySelector('[data-tasks-root]');
  root?.selectTask?.();
}
let taskEvents, taskVisible = () => true;
export function refreshTasks() { if (taskVisible()) document.querySelector('[data-tasks-root]')?.refreshTasks?.(); }
function subscribeTasks() {
  if (!taskEvents) taskEvents = Promise.all([
    listen('tasks://changed', refreshTasks),
    listen('tasks://sources_changed', refreshTasks),
    listen('managed-run-authorization://changed', refreshTasks),
    listen('tasks://error', event => { if (!taskVisible()) return; const feedback = document.querySelector('[data-task-feedback]'); if (feedback) feedback.textContent = event.payload?.message ?? '任务同步失败，请刷新'; }),
  ]).catch(() => { taskEvents = null; });
}
export async function hydrateTasks(isVisible = () => true) {
  taskVisible = isVisible;
  const root = document.querySelector('[data-tasks-root]');
  if (!root) return;
  bindChiefResources(root);
  const current = pageRequest(root);
  const message = root.querySelector('[data-task-feedback]');
  let data, selected = null, busy = false, sources = [], refreshQueued = false, readerLifetime=null, detailRevision=null, projectEditorId=null;
  let projectAuthorization = null, authorizationDraft = null, authorizationRequest = 0;
  let activeDrag = null, suppressTaskClick = null, pendingRefreshNotice = null, pendingStatusDrop = null;
  subscribeTasks();
  const projectOptions = blank => `<option value="">${blank}</option>${data.projects.map(p => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join('')}`;
  const feedback = (text, form) => {
    const local = form?.querySelector('[data-task-form-feedback], [data-task-status-feedback]');
    if (local) { local.textContent = text; local.hidden = !text; message.textContent = ''; }
    else message.textContent = text;
  };
  const detailDirty = except => [...root.querySelectorAll('[data-task-detail] input, [data-task-detail] select, [data-task-detail] textarea')].some(control => !control.closest('[data-task-version-compare]') && !except?.contains(control) && control.value !== control.dataset.savedValue);
  const authorizationDraftDirty = () => {
    if (authorizationDraft?.projectId === projectEditorId) return true;
    const form = root.querySelector('[data-task-project-history]');
    const toggle = form?.querySelector('[data-task-authorization-history]');
    return Boolean(toggle && String(toggle.checked) !== toggle.dataset.savedValue);
  };
  const projectManagerDirty = () => {
    const form = root.querySelector('[data-task-project-dependencies]');
    return authorizationDraftDirty() || Boolean(form && !form.hidden && [...form.querySelectorAll('[data-project-dependency]')]
      .some(control => String(control.checked) !== control.dataset.savedValue));
  };
  const authorizationPanel = root.querySelector('[data-task-project-authorization]');
  const authorizationRoleLabels = { planner: '规划', executor: '执行', reviewer: '复核' };
  const authorizationRolesMarkup = (selectedRoles = []) => Object.entries(authorizationRoleLabels).map(([role, label]) =>
    `<label class="task-project-authorization-role"><input type="checkbox" data-task-authorization-role="${role}"${selectedRoles.includes(role) ? ' checked' : ''}>${label}</label>`).join('');
  const paintProjectAuthorization = () => {
    if (!projectAuthorization || projectAuthorization.projectId !== projectEditorId) return;
    const view = projectAuthorization;
    if (authorizationDraft?.projectId === projectEditorId) {
      const draft = authorizationDraft;
      authorizationPanel.innerHTML = `<div class="task-project-authorization-heading"><strong>确认项目运行授权</strong><span>策略版本 ${view.policyRevision}</span></div>
        ${draft.canonicalPath ? `<p>将授权此本机目录：</p><code class="task-project-root-path">${esc(draft.canonicalPath)}</code>` : '<p>请重新选择本机项目目录，再确认本地授权。</p>'}
        <fieldset><legend>允许的项目能力</legend>${authorizationRolesMarkup(draft.roles)}</fieldset>
        <label class="task-project-history-opt-in"><input type="checkbox" data-task-authorization-history${draft.historyCaptureEnabled ? ' checked' : ''}>允许记录此项目的自有运行历史（默认关闭）</label>
        <p class="task-project-authorization-note">确认只保存本地授权，不会启动智能体。目录身份或策略状态无法核实时，授权会保持关闭。</p>
        <p class="task-project-authorization-feedback" data-task-authorization-feedback role="status" aria-live="polite"${draft.feedback ? '' : ' hidden'}>${esc(draft.feedback ?? '')}</p>
        <div class="task-project-authorization-actions"><button class="mini-btn" type="button" data-task-authorization-reselect>${draft.token ? '重新选择目录' : '选择项目目录'}</button><button class="mini-btn primary" type="button" data-task-authorization-confirm${draft.token && draft.roles.length ? '' : ' disabled'}>确认授权</button><button class="mini-btn" type="button" data-task-authorization-cancel>取消</button></div>`;
      authorizationPanel.hidden = false;
      authorizationPanel.querySelectorAll('[data-task-authorization-role]').forEach(input => {
        input.onchange = () => {
          draft.roles = [...authorizationPanel.querySelectorAll('[data-task-authorization-role]:checked')].map(control => control.dataset.taskAuthorizationRole);
          authorizationPanel.querySelector('[data-task-authorization-confirm]').disabled = !draft.token || !draft.roles.length;
        };
      });
      authorizationPanel.querySelector('[data-task-authorization-history]').onchange = event => { draft.historyCaptureEnabled = event.currentTarget.checked; };
      authorizationPanel.querySelector('[data-task-authorization-reselect]').onclick = async () => {
        if (busy) return;
        const previousToken = draft.token;
        draft.token = null;
        draft.canonicalPath = null;
        draft.feedback = '';
        draft.expectedPolicyRevision = view.policyRevision;
        if (previousToken) await invoke('task_project_authorization_cancel_consent', { token: previousToken }).catch(() => {});
        const button = authorizationPanel.querySelector('[data-task-authorization-reselect]');
        if (button) button.disabled = true;
        try {
          const candidate = await invoke('task_project_authorization_choose_root', {
            projectId: draft.projectId,
            expectedPolicyRevision: draft.expectedPolicyRevision,
          });
          if (!candidate) { authorizationDraft = null; paintProjectAuthorization(); return; }
          Object.assign(draft, candidate, { token: candidate.token });
          draft.canonicalPath = candidate.canonicalPath;
          paintProjectAuthorization();
        } catch (error) {
          draft.token = null;
          draft.canonicalPath = null;
          if (error?.code === 'project_policy_revision_conflict') {
            try {
              projectAuthorization = await invoke('task_project_authorization_snapshot', { projectId: draft.projectId });
              draft.expectedPolicyRevision = projectAuthorization.policyRevision;
            } catch {}
          }
          draft.feedback = `${error?.message ?? String(error)}；请重新选择目录后再试。`;
          paintProjectAuthorization();
        }
      };
      authorizationPanel.querySelector('[data-task-authorization-cancel]').onclick = async () => {
        if (draft.token) await invoke('task_project_authorization_cancel_consent', { token: draft.token }).catch(() => {});
        authorizationDraft = null;
        paintProjectAuthorization();
      };
      authorizationPanel.querySelector('[data-task-authorization-confirm]').onclick = async () => {
        if (busy || !draft.token || !draft.roles.length) return;
        const button = authorizationPanel.querySelector('[data-task-authorization-confirm]');
        button.disabled = true;
        try {
          const next = await invoke('task_project_authorization_confirm', {
            projectId: draft.projectId,
            candidateToken: draft.token,
            roles: draft.roles,
            historyCaptureEnabled: draft.historyCaptureEnabled,
            expectedPolicyRevision: draft.expectedPolicyRevision,
          });
          authorizationDraft = null;
          projectAuthorization = next;
          paintProjectAuthorization();
        } catch (error) {
          if (draft.token) await invoke('task_project_authorization_cancel_consent', { token: draft.token }).catch(() => {});
          draft.token = null;
          draft.canonicalPath = null;
          if (error?.code === 'project_policy_revision_conflict') {
            try {
              projectAuthorization = await invoke('task_project_authorization_snapshot', { projectId: draft.projectId });
              draft.expectedPolicyRevision = projectAuthorization.policyRevision;
            } catch {}
          }
          draft.feedback = `${error?.message ?? String(error)}；确认没有写入，请重新选择目录。`;
          paintProjectAuthorization();
        }
      };
      return;
    }

    const authorized = view.status === 'authorized';
    const needsReconfirmation = view.status === 'requires_reconfirmation';
    const roleSummary = view.roles.map(role => authorizationRoleLabels[role] ?? role).join('、') || '无';
    authorizationPanel.innerHTML = `<div class="task-project-authorization-heading"><strong>项目运行授权</strong><span>策略版本 ${view.policyRevision}</span></div>
      <p>${authorized ? '已授权' : needsReconfirmation ? '目录身份需要重新确认' : '未授权；运行保持关闭'}</p>
      ${view.rootPath ? `<code class="task-project-root-path">${esc(view.rootPath)}</code>` : ''}
      ${authorized || needsReconfirmation ? `<p class="task-project-authorization-summary">允许：${esc(roleSummary)} · 自有历史${view.historyCaptureEnabled ? '已开启' : '关闭'}</p>` : ''}
      ${needsReconfirmation ? '<p class="task-project-authorization-note">目录身份已变化或无法核实；旧授权不会生效。请重新选择目录并再次确认。</p>' : ''}
      <p class="task-project-authorization-note">此阶段只管理本地授权；不会启动智能体。项目历史采集默认关闭，策略文件没有自动导入或恢复入口。</p>
      ${authorized ? `<form data-task-project-history><label class="task-project-history-opt-in"><input type="checkbox" data-task-authorization-history${view.historyCaptureEnabled ? ' checked' : ''}>允许记录此项目的自有运行历史</label><div class="task-project-authorization-actions"><button class="mini-btn" type="submit" data-task-authorization-save-history hidden>保存历史设置</button><button class="mini-btn" type="button" data-task-authorization-edit>调整授权</button><button class="mini-btn" type="button" data-task-authorization-revoke>撤销授权</button></div><p class="task-project-authorization-feedback" data-task-authorization-feedback role="status" aria-live="polite" hidden></p></form>` : `<div class="task-project-authorization-actions"><button class="mini-btn" type="button" data-task-authorization-edit>${needsReconfirmation ? '重新选择目录并确认' : '选择目录并配置授权'}</button>${needsReconfirmation ? '<button class="mini-btn" type="button" data-task-authorization-revoke>撤销旧授权</button>' : ''}</div><p class="task-project-authorization-feedback" data-task-authorization-feedback role="status" aria-live="polite" hidden></p>`}`;
    authorizationPanel.hidden = false;
    authorizationPanel.querySelector('[data-task-authorization-edit]')?.addEventListener('click', () => {
      authorizationDraft = {
        projectId: projectEditorId,
        roles: authorized || needsReconfirmation ? [...view.roles] : [],
        historyCaptureEnabled: authorized || needsReconfirmation ? view.historyCaptureEnabled : false,
        token: null,
        canonicalPath: null,
        expectedPolicyRevision: view.policyRevision,
      };
      paintProjectAuthorization();
      authorizationPanel.querySelector('[data-task-authorization-reselect]')?.click();
    });
    const historyForm = authorizationPanel.querySelector('[data-task-project-history]');
    if (historyForm) {
      const toggle = historyForm.querySelector('[data-task-authorization-history]');
      const save = historyForm.querySelector('[data-task-authorization-save-history]');
      toggle.dataset.savedValue = String(toggle.checked);
      toggle.onchange = () => { save.hidden = String(toggle.checked) === toggle.dataset.savedValue; };
      historyForm.onsubmit = async event => {
        event.preventDefault();
        const enabled = toggle.checked;
        if (enabled && !window.confirm('开启后，此项目受管理运行产生的历史会按本地保留策略保存。是否开启？')) return;
        save.disabled = true;
        try {
          projectAuthorization = await invoke('task_project_authorization_set_history', {
            projectId: projectEditorId,
            enabled,
            expectedPolicyRevision: view.policyRevision,
          });
          paintProjectAuthorization();
        } catch (error) {
          const feedback = historyForm.querySelector('[data-task-authorization-feedback]');
          feedback.textContent = error?.message ?? String(error);
          feedback.hidden = false;
          save.disabled = false;
        }
      };
    }
    authorizationPanel.querySelector('[data-task-authorization-revoke]')?.addEventListener('click', async event => {
      if (busy || !window.confirm('撤销后，此项目的旧运行授权立即失效。确定撤销吗？')) return;
      const button = event.currentTarget;
      button.disabled = true;
      try {
        projectAuthorization = await invoke('task_project_authorization_revoke', {
          projectId: projectEditorId,
          expectedPolicyRevision: view.policyRevision,
        });
        paintProjectAuthorization();
      } catch (error) {
        const feedback = authorizationPanel.querySelector('[data-task-authorization-feedback]');
        feedback.textContent = error?.message ?? String(error);
        feedback.hidden = false;
        button.disabled = false;
      }
    });
  };
  const loadProjectAuthorization = async () => {
    if (!projectEditorId) {
      authorizationRequest++;
      authorizationPanel.hidden = true;
      authorizationPanel.innerHTML = '';
      projectAuthorization = null;
      authorizationDraft = null;
      return;
    }
    const projectId = projectEditorId;
    const request = ++authorizationRequest;
    authorizationPanel.hidden = false;
    authorizationPanel.innerHTML = '<p class="task-project-authorization-note">正在读取授权状态…</p>';
    try {
      const view = await invoke('task_project_authorization_snapshot', { projectId });
      if (request !== authorizationRequest || projectId !== projectEditorId) return;
      projectAuthorization = view;
      paintProjectAuthorization();
    } catch (error) {
      if (request !== authorizationRequest || projectId !== projectEditorId) return;
      authorizationPanel.innerHTML = `<p class="task-project-authorization-feedback" role="status">${esc(error?.message ?? String(error))}；未授予新权限。</p>`;
      authorizationPanel.hidden = false;
    }
  };
  const renderProjectManager = (force = false) => {
    if (!force && projectManagerDirty()) return;
    const manager = root.querySelector('[data-task-project-manager]');
    const select = manager.querySelector('[data-task-project-select]');
    const form = manager.querySelector('[data-task-project-dependencies]');
    const deletePanel = manager.querySelector('[data-task-project-delete]');
    form.oninput = null;
    form.onchange = null;
    form.onsubmit = null;
    const previous = select.value || projectEditorId;
    select.innerHTML = `<option value="">选择项目</option>${data.projects.map(p => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join('')}`;
    projectEditorId = data.projects.some(p => p.id === previous) ? previous : data.projects[0]?.id ?? null;
    select.value = projectEditorId ?? '';
    if (!projectEditorId) { form.hidden = true; form.innerHTML = ''; deletePanel.hidden = true; loadProjectAuthorization(); return; }
    const project = data.projects.find(p => p.id === projectEditorId);
    const dependencies = new Set(project.dependencies ?? []);
    const choices = data.projects.filter(item => item.id !== project.id);
    form.dataset.projectId = project.id;
    form.dataset.baseRevision = String(data.revision);
    form.hidden = false;
    form.innerHTML = `<fieldset><legend>${esc(project.name)} 依赖的项目</legend>${choices.length
      ? choices.map(item => `<label class="task-project-dependency"><input type="checkbox" data-project-dependency="${esc(item.id)}" value="${esc(item.id)}"${dependencies.has(item.id) ? ' checked' : ''}>${esc(item.name)}</label>`).join('')
      : '<p class="task-note">添加其他项目后，可以在这里设置依赖。</p>'}</fieldset>${choices.length ? draftActions('保存依赖') : ''}`;
    form.querySelectorAll('[data-project-dependency]').forEach(control => { control.dataset.savedValue = String(control.checked); });
    const controls = [...form.querySelectorAll('[data-project-dependency]')];
    const discard = form.querySelector('[data-task-discard]');
    if (discard) {
      discard.textContent = '撤销草稿并重新载入';
      const update = () => {
        discard.hidden = !projectManagerDirty();
        const notice = form.querySelector('[data-task-form-feedback]');
        notice.textContent = ''; notice.hidden = true;
      };
      form.oninput = update;
      form.onchange = update;
      discard.onclick = () => {
        if (busy) return;
        renderProjectManager(true);
        manager.querySelector('[data-task-project-select]').focus({ preventScroll: true });
        feedback('已重新载入当前项目记录。');
      };
      form.onsubmit = event => {
        event.preventDefault();
        const nextDependencies = controls.filter(control => control.checked).map(control => control.value);
        mutate('task_project_dependencies_update', { projectId: project.id, dependencies: nextDependencies }, form, Number(form.dataset.baseRevision));
      };
    }
    deletePanel.hidden = false;
    const taskCount = data.tasks.filter(task => task.project_id === project.id).length;
    deletePanel.querySelector('[data-project-task-count]').textContent = taskCount
      ? `有 ${taskCount} 个任务归属此项目；调整归属后才可删除。`
      : '没有任务归属此项目。';
    loadProjectAuthorization();
  };
  const leaveDetail = () => {
    if (!detailDirty()) return true;
    feedback('详情有未保存修改，请先保存或撤销。'); return false;
  };
  const reveal = (control, anchor = control, block = 'nearest') => {
    control?.focus({ preventScroll: true });
    anchor?.scrollIntoView({ block, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
  };
  const revealDetail = () => {
    const detail = root.querySelector('[data-task-detail]');
    // Explicit selection starts the inspector at its own header. Background
    // snapshots must never move the list or the enclosing workbench viewport.
    detail.scrollTop = 0;
    reveal(detail, detail.querySelector('.task-detail-heading'), 'nearest');
  };
  const closeDetail = () => {
    if (busy || !leaveDetail()) return;
    const origin = selected;
    selected = null; redraw();
    reveal([...root.querySelectorAll('[data-task-select]')].find(button => button.dataset.taskSelect === origin) ?? root.querySelector('[data-task-filter]'));
  };
  const redraw = (detail = true, forceProjectManager = false) => retainTaskFocus(root, () => {
    if (activeDrag) { refreshQueued = true; return; }
    for (const select of [root.querySelector('[data-task-filter]'), root.querySelector('[data-task-create] select')]) {
      const value = select.value;
      select.innerHTML = projectOptions(select.hasAttribute('data-task-filter') ? '全部项目' : '未分组');
      select.value = value;
    }
    renderProjectManager(forceProjectManager);
    const project = root.querySelector('[data-task-filter]').value;
    const archived = root.querySelector('[data-task-archived]').checked;
    const projectTargets = root.querySelector('[data-task-project-targets]');
    projectTargets.hidden = true;
    root.querySelector('[data-task-project-target-list]').innerHTML = [
      { id: '', name: '未分组' },
      ...data.projects.map(item => ({ id: item.id, name: item.name })),
    ].map(item => `<div role="listitem" class="task-project-target" data-task-project-target="${esc(item.id)}" aria-label="将任务移至${esc(item.name)}">${esc(item.name)}</div>`).join('');
    root.querySelector('[data-task-status-target-list]').innerHTML = Object.entries(boardColumns)
      .map(([column, label]) => `<div role="listitem" class="task-status-target" data-task-status-target="${column}" aria-label="记录为${label}">${label}</div>`).join('');
    // 跨项目搜索：按名称或来源 agent 模糊匹配，与项目过滤、归档开关组合（AND）。
    // 默认项目过滤是「全部项目」，所以空查询=原行为；有查询时想不起项目也能找到。
    const query = root.querySelector('[data-task-search]').value.trim().toLowerCase();
    const tasks = data.tasks.filter(t => Boolean(t.archived_ms != null) === archived
      && (!project || t.project_id === project)
      && (!query || t.title.toLowerCase().includes(query) || (t.source && t.source.agent_id.toLowerCase().includes(query))));
    const groups = [['attention', '需处理'], ['running', '执行中'], ['queued', '未开始'], ['finished', '已结束']];
    root.querySelector('[data-task-board]').innerHTML = tasks.length ? groups.map(([key, label]) => {
      const items = tasks.filter(t => taskGroup(data, t) === key);
      if (!items.length) return '';
      return `<section class="task-group"><h2>${label}<span>${items.length}</span></h2><div>${items.map(t => {
        const run = data.runs.find(r => r.id === t.current_run_id);
        const attention = data.attentions.find(a => a.run_id === run?.id && a.state === 'open');
        const project = data.projects.find(p => p.id === t.project_id);
        const sourceMissing = t.source && !sources.some(c => c.source.agent_id === t.source.agent_id && c.source.session_id === t.source.session_id);
        return `<button class="task-row${t.id === selected ? ' is-selected' : ''}" type="button" draggable="${t.archived_ms == null}" data-task-select="${esc(t.id)}" data-task-source-column="${key}" aria-expanded="${t.id === selected}"><span><strong>${esc(t.title)}</strong><small>${esc(project?.name ?? '未分组')}${t.source ? ` · ${esc(t.source.agent_id)}${sourceMissing ? ' · 来源暂未观测' : ''}` : ' · 本地任务'}</small></span><span class="task-state state-${key}">${esc(attention ? kinds[attention.kind] : sourceMissing && run?.status === 'running' ? '执行中 · 最后记录' : statuses[run?.status] ?? '未开始')}</span></button>`;
      }).join('')}</div></section>`;
    }).join('') : `<div class="wb-empty">${query ? `没有匹配「${esc(root.querySelector('[data-task-search]').value.trim())}」的任务` : archived ? '暂无归档任务' : project ? '这个项目还没有任务' : '还没有任务。先记录一件要做的事。'}</div>`;
    root.querySelectorAll('[data-task-select]').forEach(button => {
      button.onclick = () => {
      if (suppressTaskClick === button.dataset.taskSelect) { suppressTaskClick = null; return; }
      if (busy || !leaveDetail()) return;
      selected = selected === button.dataset.taskSelect ? null : button.dataset.taskSelect;
      redraw();
      if (selected) revealDetail();
      };
      button.addEventListener('dragstart', event => {
        const task = data.tasks.find(item => item.id === button.dataset.taskSelect);
        if (!task || task.archived_ms != null || busy) { event.preventDefault(); return; }
        if (detailDirty()) { event.preventDefault(); feedback('任务详情有未保存修改，请先保存或撤销。'); return; }
        if (projectManagerDirty()) { event.preventDefault(); feedback('项目依赖有未保存草稿，请先保存或撤销。'); return; }
        activeDrag = {
          taskId: task.id,
          taskTitle: task.title,
          sourceColumn: button.dataset.taskSourceColumn,
          expectedRevision: data.revision,
        };
        projectTargets.hidden = false;
        event.dataTransfer.effectAllowed = 'move';
        event.dataTransfer.setData('text/plain', task.id);
        root.classList.add('is-task-dragging');
      });
      button.addEventListener('dragend', () => {
        const ownsDrag = activeDrag?.taskId === button.dataset.taskSelect;
        if (ownsDrag) {
          activeDrag = null;
        }
        projectTargets.hidden = true;
        root.classList.remove('is-task-dragging');
        if (ownsDrag && refreshQueued && !busy) { refreshQueued = false; refresh(); }
        if (suppressTaskClick === button.dataset.taskSelect) return;
        suppressTaskClick = button.dataset.taskSelect;
        setTimeout(() => { if (suppressTaskClick === button.dataset.taskSelect) suppressTaskClick = null; }, 0);
      });
    });
    root.querySelectorAll('[data-task-project-target]').forEach(target => {
      target.addEventListener('dragover', event => {
        if (!activeDrag) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = 'move';
        target.classList.add('is-drop-target');
      });
      target.addEventListener('dragleave', event => {
        if (!target.contains(event.relatedTarget)) target.classList.remove('is-drop-target');
      });
      target.addEventListener('drop', event => {
        event.preventDefault();
        const drag = activeDrag;
        activeDrag = null;
        projectTargets.hidden = true;
        root.classList.remove('is-task-dragging');
        target.classList.remove('is-drop-target');
        if (!drag) return;
        if (event.dataTransfer?.getData('text/plain') !== drag.taskId) {
          feedback('拖动任务已失效，请重新选择后再试。');
          if (refreshQueued && !busy) { refreshQueued = false; refresh(); }
          return;
        }
        if (detailDirty()) { feedback('任务详情有未保存修改，请先保存或撤销。'); }
        else if (projectManagerDirty()) { feedback('项目依赖有未保存草稿，请先保存或撤销。'); }
        else {
          mutate('task_board_move_project', {
            id: drag.taskId,
            sourceColumn: drag.sourceColumn,
            projectId: target.dataset.taskProjectTarget || null,
          }, null, drag.expectedRevision);
        }
        if (refreshQueued && !busy) { refreshQueued = false; refresh(); }
      });
    });
    root.querySelectorAll('[data-task-status-target]').forEach(target => {
      target.addEventListener('dragover', event => {
        if (!activeDrag) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = 'move';
        target.classList.add('is-drop-target');
      });
      target.addEventListener('dragleave', event => {
        if (!target.contains(event.relatedTarget)) target.classList.remove('is-drop-target');
      });
      target.addEventListener('drop', event => {
        event.preventDefault();
        const drag = activeDrag;
        activeDrag = null;
        projectTargets.hidden = true;
        root.classList.remove('is-task-dragging');
        target.classList.remove('is-drop-target');
        if (!drag) return;
        if (event.dataTransfer?.getData('text/plain') !== drag.taskId) {
          feedback('拖动任务已失效，请重新选择后再试。');
          if (refreshQueued && !busy) { refreshQueued = false; refresh(); }
          return;
        }
        if (detailDirty()) { feedback('任务详情有未保存修改，请先保存或撤销。'); }
        else if (projectManagerDirty()) { feedback('项目依赖有未保存草稿，请先保存或撤销。'); }
        else if (drag.sourceColumn === target.dataset.taskStatusTarget) {
          feedback('任务已在该状态列，无需变更。');
        } else {
          const dialog = root.querySelector('[data-task-status-dialog]');
          const select = dialog.querySelector('[data-task-status-select]');
          const title = dialog.querySelector('[data-task-status-title]');
          const notice = dialog.querySelector('[data-task-status-feedback]');
          pendingStatusDrop = { ...drag, targetColumn: target.dataset.taskStatusTarget };
          title.textContent = `记录「${drag.taskTitle}」的本地进度`;
          select.innerHTML = `<option value="">选择具体进度</option>${statusChoices[pendingStatusDrop.targetColumn]
            .map(([value, status, kind, label]) => `<option value="${value}" data-status="${status}"${kind ? ` data-kind="${kind}"` : ''}>${label}</option>`).join('')}`;
          select.value = '';
          notice.textContent = ''; notice.hidden = true;
          dialog.showModal();
          select.focus({ preventScroll: true });
        }
        if (refreshQueued && !busy) { refreshQueued = false; refresh(); }
      });
    });
    if (detail) renderDetail();
  });
  const renderDetail = () => {
    root.leavePage?.();root.leavePage=null;
    const box = root.querySelector('[data-task-detail]');
    const task = data.tasks.find(t => t.id === selected);
    box.hidden = !task;
    if (!task) { box.innerHTML = ''; return; }
    const revision = data.revision;
    detailRevision = revision;
    const availableSources = sources.slice();
    const run = data.runs.find(r => r.id === task.current_run_id);
    const open = data.attentions.filter(a => a.run_id === run?.id && a.state === 'open');
    box.innerHTML = `<div class="task-detail-heading"><h2>任务详情</h2><button class="mini-btn" type="button" data-task-close>收起</button></div>
      <form data-task-edit><label>名称<input name="title" value="${esc(task.title)}" maxlength="500" required></label><label>项目<select name="projectId">${projectOptions('未分组')}</select></label>${draftActions('保存')}</form>
      <div class="task-source"><span>${task.source ? `已关联 ${esc(task.source.agent_id)}` : '尚未关联会话'}</span>${task.source ? `<button type="button" class="mini-btn" data-task-open>${task.source.thread_id && task.source.agent_id === 'codex' ? '打开会话' : '打开工具'}</button>` : ''}</div>
      ${!task.source && availableSources.length ? `<form data-task-link><label>关联来源<select name="source">${availableSources.map((c, index) => `<option value="${index}">${esc(c.name)} · ${esc(statuses[c.observation.status])}${c.target?.exactSession ? ' · 会话' : c.target ? ' · 仅打开工具' : ' · 无桌面跳转'}</option>`).join('')}</select></label>${draftActions('关联')}</form>` : !task.source ? '<p class="task-note">暂未观测到可关联的会话。工具产生明确执行状态后，刷新查看。</p>' : ''}
      ${open.map(a => `<div class="task-gate"><div><strong>${esc(kinds[a.kind])}</strong><p>${esc(data.artifacts.find(x => x.id === a.artifact_id)?.title ?? taskGateNotice(a))}</p>${artifactReadButton(data.artifacts.find(x=>x.id===a.artifact_id),run.id)}</div><button class="mini-btn" type="button" data-task-handle="${esc(a.id)}">本地已处理</button></div>`).join('')}
      <section class="task-artifact-view" data-task-artifact-view hidden aria-label="产出内容"><div class="task-detail-heading"><strong data-artifact-title>产出内容</strong><button class="mini-btn" type="button" data-artifact-close>关闭</button></div><p data-artifact-notice role="status" aria-live="polite"></p><pre data-artifact-text tabindex="0" hidden></pre></section>
      ${task.archived_ms == null ? `<form class="task-progress" data-task-progress><label>记录进度<select name="progress"><option value="running">执行中</option><option value="answer">待回答</option><option value="plan_approval">确认方案</option><option value="result_review">验收结果</option><option value="ready">结果就绪</option><option value="failed">执行失败</option><option value="cancelled">已取消</option></select></label><label>产出名称（填写正文时必填）<input name="artifactTitle" placeholder="简短说明（可选）" maxlength="500"></label><label>产出正文（可选）<textarea name="artifactContent" rows="4" placeholder="最多 64 KiB；可识别的凭据内容不会保存"></textarea></label>${draftActions('记录')}</form>` : ''}
      ${renderTaskHistory(data, task.id)}
      ${renderArtifactVersions(data, task)}
      <button class="mini-btn" type="button" data-task-archive>${task.archived_ms == null ? '归档任务' : '恢复任务'}</button>`;
    const viewer=box.querySelector('[data-task-artifact-view]');
    const body=viewer.querySelector('[data-artifact-text]'),notice=viewer.querySelector('[data-artifact-notice]');
    let readerToken=0,readerCaller=null;
    const closeReader=(restoreFocus=true)=>{
      readerLifetime?.();readerLifetime=null;readerToken++;
      body.textContent='';body.hidden=true;notice.textContent='';viewer.hidden=true;
      if(readerCaller?.isConnected){readerCaller.disabled=false;if(restoreFocus)readerCaller.focus({preventScroll:true});}
    };
    root.leavePage=()=>closeReader(false);
    viewer.querySelector('[data-artifact-close]').onclick=()=>closeReader();
    viewer.onkeydown=event=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();closeReader();}};
    box.querySelectorAll('[data-task-artifact-read]').forEach(button=>{button.onclick=async()=>{
      if(busy)return;
      busy=true;readerLifetime?.();readerLifetime=null;const token=++readerToken;readerCaller=button;const requestedAt=performance.now();
      const readButtons=[...box.querySelectorAll('[data-task-artifact-read]')];readButtons.forEach(control=>{control.disabled=true;});
      const localOutput=button.textContent==='查看产出';
      viewer.hidden=false;viewer.setAttribute('aria-busy','true');body.textContent='';body.hidden=true;notice.textContent=localOutput?'正在读取本地产出':'正在读取来源内容';
      viewer.querySelector('[data-artifact-title]').textContent=localOutput?'本地产出':button.textContent==='查看问题'?'来源问题':button.textContent==='查看方案'?'来源方案':'本轮结果';
      try {
        const result=await invoke('task_artifact_content',{id:task.id,runId:button.dataset.artifactRun,artifactId:button.dataset.taskArtifactRead,expectedRevision:revision});
        if(!current()||selected!==task.id||token!==readerToken||!viewer.isConnected)return;
        notice.textContent=result.notice;
        if(result.available){
          body.textContent=result.text??'';body.hidden=false;body.focus({preventScroll:true});
          if(result.transient_valid_for_ms!=null)readerLifetime=watchArtifactLifetime({viewer,body,notice,
            validForMs:Math.max(0,result.transient_valid_for_ms-(performance.now()-requestedAt)),
            current:()=>current()&&selected===task.id&&token===readerToken,
            validate:()=>invoke('task_artifact_content',{id:task.id,runId:button.dataset.artifactRun,artifactId:button.dataset.taskArtifactRead,expectedRevision:revision})});
        }
        else viewer.querySelector('[data-artifact-close]').focus({preventScroll:true});
        viewer.scrollIntoView({block:'nearest',behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'auto':'smooth'});
      }catch(error){if(current()&&token===readerToken&&viewer.isConnected){notice.textContent=error?.message??String(error);viewer.querySelector('[data-artifact-close]').focus({preventScroll:true});}}
      finally{busy=false;readButtons.forEach(control=>{if(control.isConnected)control.disabled=false;});viewer.removeAttribute('aria-busy');if(refreshQueued){refreshQueued=false;refresh();}}
    };});
    box.querySelectorAll('[data-task-version-compare]').forEach(form=>{form.onsubmit=async event=>{
      event.preventDefault();
      if(busy)return;
      const fromArtifactId=form.elements.fromArtifactId.value;
      const toArtifactId=form.elements.toArtifactId.value;
      const localFeedback=form.querySelector('[data-version-feedback]');
      localFeedback.textContent='';localFeedback.hidden=true;
      if(fromArtifactId===toArtifactId){localFeedback.textContent='请选择两个不同版本。';localFeedback.hidden=false;return;}
      busy=true;readerLifetime?.();readerLifetime=null;const token=++readerToken;
      const submit=form.querySelector('[type="submit"]');readerCaller=submit;
      const controls=[...root.querySelectorAll('input, select, textarea, button')];
      const disabled=controls.map(control=>control.disabled);controls.forEach(control=>{control.disabled=true;});
      viewer.hidden=false;viewer.setAttribute('aria-busy','true');body.textContent='';body.hidden=true;
      notice.textContent='正在比较本地产出版本';viewer.querySelector('[data-artifact-title]').textContent='版本比较';
      try{
        const result=await invoke('task_artifact_compare',{id:task.id,fromArtifactId,toArtifactId,expectedRevision:revision});
        if(!current()||selected!==task.id||token!==readerToken||!viewer.isConnected)return;
        const titleChange=result.titleChanged?`；名称「${result.titleFrom}」→「${result.titleTo}」`:'';
        notice.textContent=`版本 ${result.fromRevision} → ${result.toRevision}${result.crossRun?' · 跨运行':''}${titleChange}`;
        body.textContent=result.contentDiff??'';body.hidden=false;body.focus({preventScroll:true});
        viewer.scrollIntoView({block:'nearest',behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'auto':'smooth'});
      }catch(error){if(current()&&token===readerToken&&viewer.isConnected){notice.textContent=error?.message??String(error);viewer.querySelector('[data-artifact-close]').focus({preventScroll:true});}}
      finally{busy=false;controls.forEach((control,index)=>{if(control.isConnected)control.disabled=disabled[index];});viewer.removeAttribute('aria-busy');if(refreshQueued){refreshQueued=false;refresh();}}
    };});
    box.querySelector('[name="projectId"]').value = task.project_id ?? '';
    box.querySelectorAll('input, select, textarea').forEach(control => { control.dataset.savedValue = control.value; });
    box.querySelectorAll('[data-task-edit], [data-task-progress], [data-task-link], [data-task-version-form], [data-task-checklist-add], [data-task-checklist-mark]').forEach(form => {
      const controls = [...form.querySelectorAll('input, select, textarea')];
      const discard = form.querySelector('[data-task-discard]');
      const update = () => {
        discard.hidden = !controls.some(control => control.value !== control.dataset.savedValue);
        const notice = form.querySelector('[data-task-form-feedback]');
        notice.textContent = ''; notice.hidden = true;
      };
      form.addEventListener('input', update);
      form.addEventListener('change', update);
      discard.onclick = () => {
        if (busy) return;
        controls.forEach(control => { control.value = control.dataset.savedValue; });
        form.dispatchEvent(new Event('input', { bubbles: true }));
        if (data.revision !== revision && !detailDirty()) {
          // A background snapshot may update the board, but cannot rebase an
          // edited form. Only after all drafts are explicitly discarded do we
          // reveal the newer saved record and bind its actions to that version.
          const selector = form.hasAttribute('data-task-edit') ? '[data-task-edit]'
            : form.hasAttribute('data-task-progress') ? '[data-task-progress]'
              : form.hasAttribute('data-task-version-form') ? '[data-task-version-form]'
                : form.hasAttribute('data-task-checklist-add') ? '[data-task-checklist-add]'
                  : form.hasAttribute('data-task-checklist-mark') ? '[data-task-checklist-mark]' : '[data-task-link]';
          renderDetail();
          box.querySelector(`${selector} input, ${selector} select`)?.focus({ preventScroll: true });
        } else controls[0]?.focus({ preventScroll: true });
      };
    });
    box.querySelector('[data-task-close]').onclick = closeDetail;
    box.querySelector('[data-task-edit]').onsubmit = event => { event.preventDefault(); const form = event.currentTarget; mutate('task_update', { id: task.id, title: form.elements.title.value, projectId: form.elements.projectId.value || null }, form); };
    box.querySelector('[data-task-archive]').onclick = () => mutate('task_archive', { id: task.id, archived: task.archived_ms == null });
    const link = box.querySelector('[data-task-link]');
    if (link) link.onsubmit = event => { event.preventDefault(); const choice = availableSources[Number(link.elements.source.value)]; if (choice) mutate('task_link_source', { id: task.id, agent: choice.source.agent_id, sessionId: choice.source.session_id }, link); };
    const sourceButtons = [...box.querySelectorAll('[data-task-open], [data-task-run-open]')];
    for (const button of sourceButtons) button.onclick = async () => {
      if (busy) return;
      busy = true;
      const disabled = sourceButtons.map(control => control.disabled);
      sourceButtons.forEach(control => { control.disabled = true; });
      try {
        const target = await invoke('task_open_source', { id: task.id, runId: button.dataset.taskRunOpen ?? null, artifactId: null, expectedRevision: revision });
        if (current.ownsRequest()) feedback(target.hint);
      } catch (error) { if (current.ownsRequest()) feedback(error?.message ?? String(error)); }
      finally {
        busy = false;
        sourceButtons.forEach((control, index) => { control.disabled = disabled[index]; });
        if (refreshQueued) { refreshQueued = false; refresh(); }
      }
    };
    box.querySelectorAll('[data-task-handle]').forEach(button => { button.onclick = () => mutate('task_mark_handled', { id: task.id, runId: run.id, attentionId: button.dataset.taskHandle }); });
    const progress = box.querySelector('[data-task-progress]');
    if (progress) progress.onsubmit = event => { event.preventDefault(); const value = progress.elements.progress.value; const kind = kinds[value] ? value : null; const status = value === 'result_review' ? 'ready' : kind ? 'waiting' : value; mutate('task_record_progress', { id: task.id, status, kind, artifactTitle: progress.elements.artifactTitle.value.trim() || null, artifactContent: progress.elements.artifactContent.value || null }, progress); };
    box.querySelectorAll('[data-task-version-form]').forEach(form=>{form.onsubmit=event=>{
      event.preventDefault();
      mutate('task_record_artifact_version',{id:task.id,artifactId:form.dataset.artifactId,artifactTitle:form.elements.artifactTitle.value.trim(),artifactContent:form.elements.artifactContent.value},form);
    };});
    box.querySelectorAll('[data-task-checklist-add]').forEach(form=>{form.onsubmit=event=>{
      event.preventDefault();
      mutate('task_artifact_checklist_add',{id:task.id,artifactId:form.dataset.artifactId,title:form.elements.title.value.trim()},form);
    };});
    box.querySelectorAll('[data-task-checklist-mark]').forEach(form=>{form.onsubmit=event=>{
      event.preventDefault();
      mutate('task_artifact_checklist_mark',{id:task.id,artifactId:form.dataset.artifactId,itemId:form.dataset.checklistItemId,status:form.elements.status.value},form);
    };});
    box.querySelectorAll('[data-task-artifact-restore]').forEach(button=>{button.onclick=()=>mutate('task_restore_artifact_version',{id:task.id,artifactId:button.dataset.taskArtifactRestore});});
  };
  const mutate = async (command, args, form, expectedRevisionOverride = null) => {
    if (busy || !data) return;
    if (['task_board_move_project', 'task_board_change_status'].includes(command) && projectManagerDirty()) {
      feedback('项目依赖有未保存草稿，请先保存或撤销。'); return;
    }
    if (args.id === selected && detailDirty(form)) { feedback('另一处有未保存修改，请先保存或撤销。', form); return; }
    if (args.id !== selected && detailDirty(form)) { feedback('任务详情有未保存修改，请先保存或撤销。', form); return; }
    busy = true;
    const controls = [...root.querySelectorAll('input, select, textarea, button')];
    const disabled = controls.map(control => control.disabled);
    controls.forEach(control => { control.disabled = true; });
    root.setAttribute('aria-busy', 'true');
    feedback('正在保存', form);
    try {
      const next = await invoke(command, { ...args, expectedRevision: expectedRevisionOverride ?? (args.id === selected ? detailRevision : data.revision) });
      if (!current.ownsRequest()) return;
      data = next;
      pendingRefreshNotice = null;
      if (command === 'task_project_create') projectEditorId = next.projects.at(-1)?.id ?? projectEditorId;
      if (form) { form.reset(); form.dispatchEvent(new Event('input', { bubbles: true })); }
      if (command === 'task_board_change_status') {
        root.querySelector('[data-task-status-dialog]').close();
        pendingStatusDrop = null;
      }
      redraw(args.id === selected, command.startsWith('task_project_')); feedback('已保存到本机');
    } catch (error) {
      if (current.ownsRequest()) {
        const text = form && error?.code === 'stale_revision'
          ? form.hasAttribute('data-task-project-dependencies')
            ? '任务记录已更新，依赖草稿已保留。撤销草稿并重新载入后再保存。'
            : '任务已更新，草稿已保留。撤销修改后核对新记录。'
          : error?.message ?? String(error);
        if (command === 'task_board_change_status') {
          if (error?.code === 'stale_revision') {
            refreshQueued = true;
            feedback('任务记录已更新，本次拖放未应用。取消后重新拖放；已选进度已保留。', form);
          } else feedback(text, form);
        } else if (command === 'task_board_move_project') {
          refreshQueued = true;
          pendingRefreshNotice = `拖放未应用：${text}`;
          feedback(`${pendingRefreshNotice} 正在重新载入。`, form);
        } else feedback(text, form);
      }
    }
    finally { busy = false; controls.forEach((control, index) => { control.disabled = disabled[index]; }); root.removeAttribute('aria-busy'); if (refreshQueued) { refreshQueued = false; refresh(); } }
  };
  const refresh = async () => {
    if (!current() || !taskVisible()) return;
    if (activeDrag) { refreshQueued = true; return; }
    if (busy) { refreshQueued = true; return; }
    busy = true;
    try {
      const [next, choices] = await Promise.all([invoke('tasks_snapshot'), invoke('task_sources')]);
      if (!current()) return;
      data = next; sources = choices;
      if(requestedProject && !detailDirty()) {
        const id=requestedProject;requestedProject=null;
        if(data.projects.some(p=>p.id===id)) {
          selected=null;redraw();root.querySelector('[data-task-filter]').value=id;
        } else {feedback('工作空间的项目已不存在，请重新选择。');return;}
      }
      let focusRequested = false;
      if (requestedTask && !detailDirty()) {
        const task=data.tasks.find(t=>t.id===requestedTask);
        if (task) { focusRequested=true; selected=task.id; root.querySelector('[data-task-filter]').value=''; root.querySelector('[data-task-archived]').checked=task.archived_ms!=null; }
        requestedTask=null;
      }
      redraw(focusRequested || !detailDirty());
      if (focusRequested) revealDetail();
      root.dataset.pageReady = 'true';
      const notice = pendingRefreshNotice;
      pendingRefreshNotice = null;
      feedback(notice ?? '');
    } catch (error) { if (current()) feedback(error?.message ?? String(error)); }
    finally { busy = false; if (refreshQueued) { refreshQueued = false; refresh(); } }
  };
  root.selectTask = () => {
    if (detailDirty()) { feedback('详情有未保存修改，请先保存或撤销。'); return; }
    refresh();
  };
  root.refreshTasks = refresh;
  root.onkeydown = event => {
    if (event.key !== 'Escape' || event.isComposing || event.defaultPrevented || event.target.tagName === 'SELECT') return;
    if (!selected || !root.querySelector('[data-task-detail]').contains(event.target)) return;
    event.preventDefault();
    closeDetail();
  };
  root.querySelector('[data-task-refresh]').onclick = refresh;
  const projectSelect = root.querySelector('[data-task-project-select]');
  projectSelect.onchange = () => {
    if (projectManagerDirty()) {
      projectSelect.value = projectEditorId ?? '';
      feedback('项目依赖有未保存草稿，请先保存或撤销。');
      return;
    }
    projectEditorId = projectSelect.value || null;
    renderProjectManager(true);
  };
  root.querySelector('[data-task-project-delete-button]').onclick = () => {
    if (!projectEditorId || busy) return;
    if (projectManagerDirty()) { feedback('项目依赖有未保存草稿，请先保存或撤销。'); return; }
    mutate('task_project_delete', { projectId: projectEditorId });
  };
  const statusDialog = root.querySelector('[data-task-status-dialog]');
  const statusForm = root.querySelector('[data-task-status-form]');
  const statusSelect = root.querySelector('[data-task-status-select]');
  const statusFeedback = root.querySelector('[data-task-status-feedback]');
  const closeStatusDialog = () => { pendingStatusDrop = null; statusFeedback.textContent = ''; statusFeedback.hidden = true; };
  root.querySelector('[data-task-status-cancel]').onclick = () => statusDialog.close();
  statusDialog.addEventListener('close', () => { if (!statusDialog.open) closeStatusDialog(); });
  statusForm.onsubmit = event => {
    event.preventDefault();
    const drop = pendingStatusDrop;
    const choice = statusSelect.selectedOptions[0];
    if (!drop || !choice?.dataset.status) {
      statusFeedback.textContent = '请选择具体进度。'; statusFeedback.hidden = false; statusSelect.focus(); return;
    }
    mutate('task_board_change_status', {
      id: drop.taskId,
      sourceColumn: drop.sourceColumn,
      targetColumn: drop.targetColumn,
      status: choice.dataset.status,
      kind: choice.dataset.kind || null,
    }, statusForm, drop.expectedRevision);
  };
  root.querySelector('[data-task-filter]').onchange = () => { if (data) redraw(false); };
  root.querySelector('[data-task-archived]').onchange = () => { if (data) redraw(false); };
  // 搜索即输即筛：纯前端过滤（板上的数据本来就是全量的），不触发网络请求。
  root.querySelector('[data-task-search]').oninput = () => { if (data) redraw(false); };
  root.querySelector('[data-task-create]').onsubmit = event => { event.preventDefault(); const form = event.currentTarget; mutate('task_create', { title: form.elements.title.value, projectId: form.elements.projectId.value || null }, form); };
  root.querySelector('[data-task-project]').onsubmit = event => {
    event.preventDefault();
    const form = event.currentTarget;
    if (projectManagerDirty()) { feedback('请先保存或撤销项目依赖草稿，再添加项目。'); return; }
    mutate('task_project_create', { name: form.elements.name.value }, form);
  };
  await refresh();
}
