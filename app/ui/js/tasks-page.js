import { watchArtifactLifetime } from './artifact-lifetime.js';
import { invoke, listen } from './tauri.js';
import { pageRequest } from './page-host.js';
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const statuses = { queued: '未开始', running: '执行中', waiting: '等待处理', ready: '结果就绪', failed: '执行失败', cancelled: '已取消', accepted: '来源已验收' };
const kinds = { answer: '待回答', plan_approval: '确认方案', result_review: '验收结果', confirmation: '需要确认' };
export function taskGateNotice(attention) {
  if (!attention.observed) return '本地记录，处理后标记即可。';
  return attention.kind === 'plan_approval' ? '来源未提供方案正文，请在工具中查看。' : '请在来源工具中处理，再更新本地记录。';
}
const draftActions = () => '<button class="mini-btn" type="button" data-task-discard hidden>撤销修改</button><p class="task-form-feedback" data-task-form-feedback role="status" aria-live="polite" hidden></p>';
export function runSourceAction(run) {
  if (!run.source) return null;
  return { runId: run.id, label: run.source.thread_id && run.source.agent_id === 'codex' ? '打开来源会话' : '打开来源工具' };
}
export function artifactReadAction(artifact, runId) {
  return artifact?.event_ref ? { artifactId: artifact.id, runId, label:artifact.kind === 'answer' ? '查看问题' : artifact.kind === 'plan_approval' ? '查看方案' : '查看结果' } : null;
}
function artifactReadButton(artifact, runId) {
  const action=artifactReadAction(artifact,runId);
  return action ? `<button class="mini-btn" type="button" data-task-artifact-read="${esc(action.artifactId)}" data-artifact-run="${esc(action.runId)}">${esc(action.label)}</button>` : '';
}
export function renderTaskHistory(data, taskId) {
  const history = data.runs.filter(r => r.task_id === taskId).slice().reverse();
  return `<details class="task-history"><summary>运行记录 · ${history.length}</summary>${history.length ? history.map((r, i) => {
    const action = runSourceAction(r);
    return `<article><div class="task-history-heading"><strong>第 ${history.length - i} 次 · ${esc(statuses[r.status])}</strong>${action ? `<button type="button" class="mini-btn" data-task-run-open="${esc(action.runId)}">${esc(action.label)}</button>` : '<span class="task-history-local">未保存来源</span>'}</div><time>${esc(new Date(r.started_ms).toLocaleString())}</time>${data.artifacts.filter(a => a.run_id === r.id && !data.attentions.some(g => g.artifact_id === a.id)).map(a => `<p class="task-artifact-line"><span>产出引用 · ${esc(a.title)}</span>${artifactReadButton(a,r.id)}</p>`).join('')}${data.attentions.filter(a => a.run_id === r.id).map(a => `<p>${esc(kinds[a.kind])} · ${a.state === 'expired' ? '已过期' : a.state === 'open' ? '待处理' : a.manual_resolution ? '本地已处理' : '来源已处理'}${a.artifact_id ? ` · ${esc(data.artifacts.find(x => x.id === a.artifact_id)?.title)}` : ''}${artifactReadButton(data.artifacts.find(x => x.id === a.artifact_id),r.id)}</p>`).join('')}</article>`;
  }).join('') : '<p>尚无运行记录</p>'}</details>`;
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
    <div class="task-toolbar"><label class="task-filter">项目<select data-task-filter aria-label="筛选项目"><option value="">全部项目</option></select></label><label class="task-archive-filter"><input type="checkbox" data-task-archived>已归档</label><button class="mini-btn" type="button" data-task-refresh>刷新</button></div>
    <form class="task-compose" data-task-create><label class="sr-only" for="task-new-title">任务名称</label><input id="task-new-title" name="title" placeholder="记录一个任务…" maxlength="500" required autocomplete="off"><select name="projectId" aria-label="任务所属项目"><option value="">未分组</option></select><button class="mini-btn primary" type="submit">添加任务</button></form>
    <details class="task-project-create"><summary>管理项目</summary><form data-task-project><input name="name" aria-label="新项目名称" placeholder="项目名称" maxlength="500" required autocomplete="off"><button class="mini-btn" type="submit">添加项目</button></form></details>
    <p class="task-feedback" data-task-feedback role="status" aria-live="polite">正在读取任务</p>
    <div data-task-board></div><section class="task-detail" tabindex="-1" data-task-detail hidden aria-label="任务详情"></section>
    <p class="task-note">在本机整理任务与运行记录。记录进度不会启动工具；标记已处理不会向工具发送批准。</p>
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
    listen('tasks://error', event => { if (!taskVisible()) return; const feedback = document.querySelector('[data-task-feedback]'); if (feedback) feedback.textContent = event.payload?.message ?? '任务同步失败，请刷新'; }),
  ]).catch(() => { taskEvents = null; });
}
export async function hydrateTasks(isVisible = () => true) {
  taskVisible = isVisible;
  const root = document.querySelector('[data-tasks-root]');
  if (!root) return;
  const current = pageRequest(root);
  const message = root.querySelector('[data-task-feedback]');
  let data, selected = null, busy = false, sources = [], refreshQueued = false, readerLifetime=null, detailRevision=null;
  subscribeTasks();
  const projectOptions = blank => `<option value="">${blank}</option>${data.projects.map(p => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join('')}`;
  const feedback = (text, form) => {
    const local = form?.querySelector('[data-task-form-feedback]');
    if (local) { local.textContent = text; local.hidden = !text; message.textContent = ''; }
    else message.textContent = text;
  };
  const detailDirty = except => [...root.querySelectorAll('[data-task-detail] input, [data-task-detail] select')].some(control => !except?.contains(control) && control.value !== control.dataset.savedValue);
  const leaveDetail = () => {
    if (!detailDirty()) return true;
    feedback('详情有未保存修改，请先保存或撤销。'); return false;
  };
  const redraw = (detail = true) => {
    for (const select of [root.querySelector('[data-task-filter]'), root.querySelector('[data-task-create] select')]) {
      const value = select.value;
      select.innerHTML = projectOptions(select.hasAttribute('data-task-filter') ? '全部项目' : '未分组');
      select.value = value;
    }
    const project = root.querySelector('[data-task-filter]').value;
    const archived = root.querySelector('[data-task-archived]').checked;
    const tasks = data.tasks.filter(t => Boolean(t.archived_ms != null) === archived && (!project || t.project_id === project));
    const groups = [['attention', '需处理'], ['running', '执行中'], ['queued', '未开始'], ['finished', '已结束']];
    root.querySelector('[data-task-board]').innerHTML = tasks.length ? groups.map(([key, label]) => {
      const items = tasks.filter(t => taskGroup(data, t) === key);
      if (!items.length) return '';
      return `<section class="task-group"><h2>${label}<span>${items.length}</span></h2><div>${items.map(t => {
        const run = data.runs.find(r => r.id === t.current_run_id);
        const attention = data.attentions.find(a => a.run_id === run?.id && a.state === 'open');
        const project = data.projects.find(p => p.id === t.project_id);
        const sourceMissing = t.source && !sources.some(c => c.source.agent_id === t.source.agent_id && c.source.session_id === t.source.session_id);
        return `<button class="task-row${t.id === selected ? ' is-selected' : ''}" type="button" data-task-select="${esc(t.id)}" aria-expanded="${t.id === selected}"><span><strong>${esc(t.title)}</strong><small>${esc(project?.name ?? '未分组')}${t.source ? ` · ${esc(t.source.agent_id)}${sourceMissing ? ' · 来源暂未观测' : ''}` : ' · 本地任务'}</small></span><span class="task-state state-${key}">${esc(attention ? kinds[attention.kind] : sourceMissing && run?.status === 'running' ? '执行中 · 最后记录' : statuses[run?.status] ?? '未开始')}</span></button>`;
      }).join('')}</div></section>`;
    }).join('') : `<div class="wb-empty">${archived ? '暂无归档任务' : project ? '这个项目还没有任务' : '还没有任务。先记录一件要做的事。'}</div>`;
    root.querySelectorAll('[data-task-select]').forEach(button => { button.onclick = () => { if (busy || !leaveDetail()) return; selected = selected === button.dataset.taskSelect ? null : button.dataset.taskSelect; redraw(); }; });
    if (detail) renderDetail();
  };
  const renderDetail = () => {
    readerLifetime?.();readerLifetime=null;
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
      <form data-task-edit><label>名称<input name="title" value="${esc(task.title)}" maxlength="500" required></label><label>项目<select name="projectId">${projectOptions('未分组')}</select></label><button class="mini-btn" type="submit">保存</button>${draftActions()}</form>
      <div class="task-source"><span>${task.source ? `已关联 ${esc(task.source.agent_id)}` : '尚未关联会话'}</span>${task.source ? `<button type="button" class="mini-btn" data-task-open>${task.source.thread_id && task.source.agent_id === 'codex' ? '打开会话' : '打开工具'}</button>` : ''}</div>
      ${!task.source && availableSources.length ? `<form data-task-link><label>关联来源<select name="source">${availableSources.map((c, index) => `<option value="${index}">${esc(c.name)} · ${esc(statuses[c.observation.status])}${c.target?.exactSession ? ' · 会话' : c.target ? ' · 仅打开工具' : ' · 无桌面跳转'}</option>`).join('')}</select></label><button class="mini-btn" type="submit">关联</button>${draftActions()}</form>` : !task.source ? '<p class="task-note">暂未观测到可关联的会话。工具产生明确执行状态后，刷新查看。</p>' : ''}
      ${open.map(a => `<div class="task-gate"><div><strong>${esc(kinds[a.kind])}</strong><p>${esc(data.artifacts.find(x => x.id === a.artifact_id)?.title ?? taskGateNotice(a))}</p>${artifactReadButton(data.artifacts.find(x=>x.id===a.artifact_id),run.id)}</div><button class="mini-btn" type="button" data-task-handle="${esc(a.id)}">本地已处理</button></div>`).join('')}
      <section class="task-artifact-view" data-task-artifact-view hidden aria-label="来源内容"><div class="task-detail-heading"><strong data-artifact-title>来源内容</strong><button class="mini-btn" type="button" data-artifact-close>关闭</button></div><p data-artifact-notice role="status" aria-live="polite"></p><pre data-artifact-text tabindex="0" hidden></pre></section>
      ${task.archived_ms == null ? `<form class="task-progress" data-task-progress><label>记录进度<select name="progress"><option value="running">执行中</option><option value="answer">待回答</option><option value="plan_approval">确认方案</option><option value="result_review">验收结果</option><option value="ready">结果就绪</option><option value="failed">执行失败</option><option value="cancelled">已取消</option></select></label><label>产出引用<input name="artifactTitle" placeholder="简短说明（可选）" maxlength="500"></label><button class="mini-btn" type="submit">记录</button>${draftActions()}</form>` : ''}
      ${renderTaskHistory(data, task.id)}
      <button class="mini-btn" type="button" data-task-archive>${task.archived_ms == null ? '归档任务' : '恢复任务'}</button>`;
    const viewer=box.querySelector('[data-task-artifact-view]');
    const body=viewer.querySelector('[data-artifact-text]'),notice=viewer.querySelector('[data-artifact-notice]');
    let readerToken=0,readerCaller=null;
    const closeReader=()=>{readerLifetime?.();readerLifetime=null;readerToken++;body.textContent='';body.hidden=true;notice.textContent='';viewer.hidden=true;if(readerCaller?.isConnected){readerCaller.disabled=false;readerCaller.focus({preventScroll:true});}};
    viewer.querySelector('[data-artifact-close]').onclick=closeReader;
    viewer.onkeydown=event=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();closeReader();}};
    box.querySelectorAll('[data-task-artifact-read]').forEach(button=>{button.onclick=async()=>{
      if(busy)return;
      busy=true;readerLifetime?.();readerLifetime=null;const token=++readerToken;readerCaller=button;const requestedAt=performance.now();
      const readButtons=[...box.querySelectorAll('[data-task-artifact-read]')];readButtons.forEach(control=>{control.disabled=true;});
      viewer.hidden=false;viewer.setAttribute('aria-busy','true');body.textContent='';body.hidden=true;notice.textContent='正在读取来源内容';
      viewer.querySelector('[data-artifact-title]').textContent=button.textContent==='查看问题'?'来源问题':button.textContent==='查看方案'?'来源方案':'本轮结果';
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
    box.querySelector('[name="projectId"]').value = task.project_id ?? '';
    box.querySelectorAll('input, select').forEach(control => { control.dataset.savedValue = control.value; });
    box.querySelectorAll('[data-task-edit], [data-task-progress], [data-task-link]').forEach(form => {
      const controls = [...form.querySelectorAll('input, select')];
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
            : form.hasAttribute('data-task-progress') ? '[data-task-progress]' : '[data-task-link]';
          renderDetail();
          box.querySelector(`${selector} input, ${selector} select`)?.focus({ preventScroll: true });
        } else controls[0]?.focus({ preventScroll: true });
      };
    });
    box.querySelector('[data-task-close]').onclick = () => { if (!leaveDetail()) return; selected = null; redraw(); };
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
    if (progress) progress.onsubmit = event => { event.preventDefault(); const value = progress.elements.progress.value; const kind = kinds[value] ? value : null; const status = value === 'result_review' ? 'ready' : kind ? 'waiting' : value; mutate('task_record_progress', { id: task.id, status, kind, artifactTitle: progress.elements.artifactTitle.value.trim() || null }, progress); };
  };
  const mutate = async (command, args, form) => {
    if (busy || !data) return;
    if (args.id === selected && detailDirty(form)) { feedback('另一处有未保存修改，请先保存或撤销。', form); return; }
    busy = true;
    const controls = [...root.querySelectorAll('input, select, button')];
    const disabled = controls.map(control => control.disabled);
    controls.forEach(control => { control.disabled = true; });
    root.setAttribute('aria-busy', 'true');
    feedback('正在保存', form);
    try {
      const next = await invoke(command, { ...args, expectedRevision: args.id === selected ? detailRevision : data.revision });
      if (!current.ownsRequest()) return;
      data = next;
      if (form) { form.reset(); form.dispatchEvent(new Event('input', { bubbles: true })); }
      redraw(args.id === selected); feedback('已保存到本机');
    } catch (error) {
      if (current.ownsRequest()) feedback(form && error?.code === 'stale_revision'
        ? '任务已更新，草稿已保留。撤销修改后核对新记录。'
        : error?.message ?? String(error), form);
    }
    finally { busy = false; controls.forEach((control, index) => { control.disabled = disabled[index]; }); root.removeAttribute('aria-busy'); if (refreshQueued) { refreshQueued = false; refresh(); } }
  };
  const refresh = async () => {
    if (!current() || !taskVisible()) return;
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
      const detailHasFocus = root.querySelector('[data-task-detail]').contains(document.activeElement);
      redraw(focusRequested || (!detailDirty() && !detailHasFocus));
      if (focusRequested) { const detail=root.querySelector('[data-task-detail]'); detail.focus({preventScroll:true}); detail.scrollIntoView({block:'nearest',behavior:matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth'}); }
      root.dataset.pageReady = 'true'; feedback('');
    } catch (error) { if (current()) feedback(error?.message ?? String(error)); }
    finally { busy = false; if (refreshQueued) { refreshQueued = false; refresh(); } }
  };
  root.selectTask = () => {
    if (detailDirty()) { feedback('详情有未保存修改，请先保存或撤销。'); return; }
    refresh();
  };
  root.refreshTasks = refresh;
  root.querySelector('[data-task-refresh]').onclick = refresh;
  root.querySelector('[data-task-filter]').onchange = () => { if (data) redraw(false); };
  root.querySelector('[data-task-archived]').onchange = () => { if (data) redraw(false); };
  root.querySelector('[data-task-create]').onsubmit = event => { event.preventDefault(); const form = event.currentTarget; mutate('task_create', { title: form.elements.title.value, projectId: form.elements.projectId.value || null }, form); };
  root.querySelector('[data-task-project]').onsubmit = event => { event.preventDefault(); const form = event.currentTarget; mutate('task_project_create', { name: form.elements.name.value }, form); };
  await refresh();
}
