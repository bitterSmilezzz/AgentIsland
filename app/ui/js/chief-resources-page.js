import { invoke, listen } from './tauri.js';
import { pageRequest } from './page-host.js';

const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const fields = [
  ['name', '配置名称'], ['role_name', '角色名称'], ['required_capabilities', '角色所需能力'],
  ['granted_scopes', '声明的角色记忆范围'], ['runtime_name', '运行时名称'],
  ['runtime_capabilities', '运行时支持的能力'], ['max_concurrency', '运行时并发上限'],
  ['model_name', '模型名称'], ['model_capabilities', '模型支持的能力'], ['memory_scope', '使用的记忆范围'],
];
const arrayFields = new Set(['required_capabilities', 'granted_scopes', 'runtime_capabilities', 'model_capabilities']);
const list = text => text.split(/\n|,/).map(s => s.trim()).filter(Boolean);

export function chiefResourcesPage() {
  return `<section class="chief-resources" data-chief-resources hidden aria-label="团队资源配置" tabindex="-1">
    <div class="chief-resources-heading"><h2>团队资源配置</h2><button class="mini-btn" type="button" data-chief-back>返回任务</button></div>
    <p>能力和记忆范围由您声明，保存不代表已验证实际能力，也不授予记忆访问权限；配置不会启动执行。请勿填写密钥或命令。</p>
    <div class="chief-resources-toolbar"><button class="mini-btn" type="button" data-chief-refresh>刷新配置</button><button class="mini-btn" type="button" data-chief-new disabled>添加配置</button></div>
    <p role="status" aria-live="polite" data-chief-status>尚未读取配置。</p>
    <div data-chief-list></div>
    <form data-chief-form hidden><h3 data-chief-form-title>添加配置</h3><p>能力和声明的记忆范围可用逗号分隔；名称与使用的记忆范围必须填写。</p>
      <div class="chief-resources-fields">${fields.map(([name,label]) => `<label><span>${label}</span><input name="${name}" aria-label="${label}" ${name === 'max_concurrency' ? 'type="number" min="1" step="1" value="1"' : 'type="text" maxlength="500" autocomplete="off"'} ${arrayFields.has(name) ? 'placeholder="逗号分隔"' : 'required'}></label>`).join('')}</div>
      <div class="chief-resources-toolbar"><button class="mini-btn primary" type="submit" data-chief-save>保存配置</button><button class="mini-btn" type="button" data-chief-cancel>取消编辑</button><button class="mini-btn" type="button" data-chief-rebase hidden>使用最新版本继续编辑</button></div>
      <p role="status" aria-live="polite" data-chief-form-status></p>
    </form>
    <div data-chief-delete hidden role="group" aria-label="删除配置确认"><p data-chief-delete-label></p><button class="mini-btn" type="button" data-chief-delete-confirm>确认删除</button> <button class="mini-btn" type="button" data-chief-delete-cancel>取消</button></div>
  </section>`;
}

export function bindChiefResources(taskRoot) {
  const root = taskRoot.querySelector('[data-chief-resources]');
  if (!root || root.dataset.bound) return;
  root.dataset.bound = 'true';
  const panes = taskRoot.querySelector('.task-panes');
  const footer = taskRoot.querySelector('.task-footer');
  const form = root.querySelector('[data-chief-form]');
  const status = root.querySelector('[data-chief-status]');
  const formStatus = root.querySelector('[data-chief-form-status]');
  const rows = root.querySelector('[data-chief-list]');
  const confirm = root.querySelector('[data-chief-delete]');
  const rebase = root.querySelector('[data-chief-rebase]');
  const current = pageRequest(root);
  let snapshot = null, baseRevision = null, editing = null, dirty = false, busy = false, stale = false, deleteId = null, requestId = 0, refreshQueued = false;
  const active = () => current() && !root.hidden;
  const feedback = text => { status.textContent = text; };
  const rowButton = (kind, id) => [...rows.querySelectorAll(`[data-chief-${kind}]`)].find(button => button.dataset[kind === 'edit' ? 'chiefEdit' : 'chiefRemove'] === id);
  const focusFallback = (kind, id) => (id && rowButton(kind, id)) || root.querySelector('[data-chief-new]:not(:disabled)') || root.querySelector('[data-chief-refresh]');
  const setBusy = value => {
    busy = value; root.setAttribute('aria-busy', String(value));
    root.querySelectorAll('button,input').forEach(el => { el.disabled = value; });
    root.querySelector('[data-chief-new]').disabled = value || !snapshot;
    root.querySelector('[data-chief-save]').disabled = value || !snapshot || stale;
  };
  const showRows = () => {
    const focused = document.activeElement;
    const kind = rows.contains(focused) ? (focused.hasAttribute('data-chief-edit') ? 'edit' : focused.hasAttribute('data-chief-remove') ? 'remove' : null) : null;
    const id = kind ? focused.dataset[kind === 'edit' ? 'chiefEdit' : 'chiefRemove'] : null;
    const values = items => items.length ? items.map(esc).join('、') : '无';
    rows.innerHTML = snapshot.profiles.length ? snapshot.profiles.map(p => `<article class="chief-resource-row"><div><strong>${esc(p.name)}</strong><p>角色 ${esc(p.role.name)} · 所需能力 ${values(p.role.required_capabilities)} · 声明的角色记忆范围 ${values(p.role.granted_scopes)}</p><p>运行时 ${esc(p.runtime.name)} · 支持能力 ${values(p.runtime.capabilities)} · 并发上限 ${esc(p.runtime.max_concurrency)}</p><p>模型 ${esc(p.model.name)} · 支持能力 ${values(p.model.capabilities)} · 使用的记忆范围 ${esc(p.memory_scope)}</p></div><div class="chief-resources-toolbar"><button class="mini-btn" type="button" data-chief-edit="${esc(p.id)}">编辑</button><button class="mini-btn" type="button" data-chief-remove="${esc(p.id)}">删除</button></div></article>`).join('') : '<p>还没有团队资源配置。</p>';
    if (kind && active()) focusFallback(kind, id)?.focus({preventScroll:true});
  };
  const closeForm = () => { form.hidden = true; form.reset(); editing = null; dirty = false; stale = false; rebase.hidden = true; formStatus.textContent = ''; };
  const openForm = profile => {
    if (dirty) { formStatus.textContent = '请先保存或取消当前编辑。'; return; }
    editing = profile?.id ?? ''; baseRevision = snapshot.revision; stale = false; dirty = false;
    form.reset();
    const values = profile ? { name: profile.name, role_name: profile.role.name, required_capabilities: profile.role.required_capabilities.join(', '), granted_scopes: profile.role.granted_scopes.join(', '), runtime_name: profile.runtime.name, runtime_capabilities: profile.runtime.capabilities.join(', '), max_concurrency: profile.runtime.max_concurrency, model_name: profile.model.name, model_capabilities: profile.model.capabilities.join(', '), memory_scope: profile.memory_scope } : {};
    for (const [name] of fields) if (values[name] !== undefined) form.elements.namedItem(name).value = values[name];
    root.querySelector('[data-chief-form-title]').textContent = profile ? '编辑配置' : '添加配置';
    form.hidden = false; rebase.hidden = true; formStatus.textContent = ''; form.elements.namedItem('name').focus();
  };
  async function read({ fromEvent = false } = {}) {
    if (!active() || busy) return;
    const id = ++requestId;
    if (!fromEvent) feedback('正在读取配置');
    try {
      const next = await invoke('chief_resources_snapshot');
      if (!active() || id !== requestId) return;
      snapshot = next;
      if (!form.hidden && next.revision !== baseRevision) {
        stale = true; rebase.hidden = false;
        formStatus.textContent = '配置已在别处更新，草稿已保留。请核对上方最新配置后，选择使用最新版本继续编辑。';
      } else if (form.hidden) { stale = false; }
      const closeConfirmation = !confirm.hidden && next.revision !== baseRevision;
      const confirmationFocused = closeConfirmation && confirm.contains(document.activeElement);
      const priorDeleteId = deleteId;
      if (closeConfirmation) { confirm.hidden = true; deleteId = null; }
      showRows(); setBusy(false);
      if (confirmationFocused && active()) focusFallback('remove', priorDeleteId)?.focus({preventScroll:true});
      feedback('已读取本机配置。');
    } catch (error) {
      if (!active() || id !== requestId) return;
      // A failed read cannot be treated as an empty or writable snapshot.
      const lostFocus = rows.contains(document.activeElement) || confirm.contains(document.activeElement);
      snapshot = null; rows.innerHTML = ''; confirm.hidden = true; deleteId = null; setBusy(false);
      if (lostFocus && active()) root.querySelector('[data-chief-refresh]').focus({preventScroll:true});
      feedback(error?.message ?? String(error));
    }
  }
  const mutate = async (command, args) => {
    if (!active() || busy || !snapshot || stale) return;
    const origin = document.activeElement;
    const fromForm = form.contains(origin), fromConfirmation = confirm.contains(origin);
    const previousIds = new Set(snapshot.profiles.map(p => p.id));
    let focusTarget = null, focusChanged = false;
    const noteFocusChange = event => { if (event.target !== origin) focusChanged = true; };
    document.addEventListener('focusin', noteFocusChange, true);
    document.addEventListener('pointerdown', noteFocusChange, true);
    ++requestId; setBusy(true);
    try {
      const next = await invoke(command, args);
      if (!current.ownsRequest()) return;
      const mayRestore = !focusChanged && active() && (document.activeElement === document.body || document.activeElement === origin || (fromForm && form.contains(document.activeElement)) || (fromConfirmation && confirm.contains(document.activeElement)));
      snapshot = next; closeForm(); confirm.hidden = true; deleteId = null; showRows(); feedback('已保存到本机。');
      const savedId = command === 'chief_resources_save' ? args.profile.id || next.profiles.find(p => !previousIds.has(p.id))?.id : null;
      if (mayRestore && (fromForm || fromConfirmation)) focusTarget = () => focusFallback('edit', savedId);
    } catch (error) {
      if (!current.ownsRequest()) return;
      if (error?.code === 'stale_revision') {
        stale = true; formStatus.textContent = '配置已更新，草稿已保留。请刷新并核对最新配置。';
        feedback('配置已更新，请刷新后再操作。');
      } else if (!form.hidden) formStatus.textContent = error?.message ?? String(error);
      else feedback(error?.message ?? String(error));
      const mayRestore = !focusChanged && active() && (document.activeElement === document.body || document.activeElement === origin || (fromForm && form.contains(document.activeElement)) || (fromConfirmation && confirm.contains(document.activeElement)));
      if (mayRestore && (fromForm || fromConfirmation)) focusTarget = () => stale ? root.querySelector('[data-chief-refresh]') : (!origin.disabled && origin.isConnected ? origin : root.querySelector('[data-chief-refresh]'));
    } finally {
      document.removeEventListener('focusin', noteFocusChange, true);
      document.removeEventListener('pointerdown', noteFocusChange, true);
      if (current.ownsRequest()) {
        setBusy(false);
        if (focusTarget && active()) focusTarget()?.focus({preventScroll:true});
        if (refreshQueued) { refreshQueued = false; read({ fromEvent: true }); }
      }
    }
  };
  taskRoot.querySelector('[data-chief-open]').onclick = () => {
    panes.hidden = true; footer.hidden = true; root.hidden = false; root.focus({preventScroll:true});
    read();
  };
  root.querySelector('[data-chief-back]').onclick = () => {
    root.hidden = true; panes.hidden = false; footer.hidden = false; ++requestId;
    taskRoot.querySelector('[data-chief-open]').focus({preventScroll:true});
  };
  root.querySelector('[data-chief-refresh]').onclick = () => read();
  root.querySelector('[data-chief-new]').onclick = () => openForm(null);
  root.querySelector('[data-chief-cancel]').onclick = () => { const shouldFocus = form.contains(document.activeElement), id = editing; closeForm(); setBusy(false); if (shouldFocus && active()) focusFallback('edit', id)?.focus({preventScroll:true}); };
  rebase.onclick = () => {
    if (!snapshot || !stale) return;
    if (editing && !snapshot.profiles.some(p => p.id === editing)) { formStatus.textContent = '此配置已被删除，请取消编辑后新建。'; return; }
    const shouldFocus = document.activeElement === rebase;
    baseRevision = snapshot.revision; stale = false; rebase.hidden = true; formStatus.textContent = '已选择最新版本；请核对草稿后保存。'; setBusy(false);
    if (shouldFocus && active()) root.querySelector('[data-chief-save]').focus({preventScroll:true});
  };
  form.oninput = () => { dirty = true; };
  form.onsubmit = event => {
    event.preventDefault();
    const value = name => form.elements.namedItem(name).value.trim();
    const profile = { id: editing, name: value('name'), role: { name: value('role_name'), required_capabilities: list(value('required_capabilities')), granted_scopes: list(value('granted_scopes')) }, runtime: { name: value('runtime_name'), capabilities: list(value('runtime_capabilities')), max_concurrency: Number(value('max_concurrency')) }, model: { name: value('model_name'), capabilities: list(value('model_capabilities')) }, memory_scope: value('memory_scope') };
    mutate('chief_resources_save', { profile, expectedRevision: baseRevision });
  };
  rows.onclick = event => {
    const edit = event.target.closest('[data-chief-edit]'), remove = event.target.closest('[data-chief-remove]');
    if (edit) { openForm(snapshot.profiles.find(p => p.id === edit.dataset.chiefEdit)); return; }
    if (remove && !dirty) { deleteId = remove.dataset.chiefRemove; baseRevision = snapshot.revision; confirm.querySelector('[data-chief-delete-label]').textContent = `删除「${snapshot.profiles.find(p => p.id === deleteId)?.name ?? ''}」？`; confirm.hidden = false; confirm.querySelector('[data-chief-delete-confirm]').focus(); }
    else if (remove) formStatus.textContent = '请先保存或取消当前编辑。';
  };
  root.querySelector('[data-chief-delete-cancel]').onclick = () => { const shouldFocus = confirm.contains(document.activeElement), id = deleteId; confirm.hidden = true; deleteId = null; if (shouldFocus && active()) focusFallback('remove', id)?.focus({preventScroll:true}); };
  root.querySelector('[data-chief-delete-confirm]').onclick = () => mutate('chief_resources_delete', { id: deleteId, expectedRevision: baseRevision });
  // A cached page can leave and return before a prior read resolves. Invalidate
  // that read when leaving and start a new one after the same DOM is restored.
  root.chiefSuspend = () => { ++requestId; };
  root.chiefResume = () => {
    if (root.hidden) return;
    if (busy) { refreshQueued = true; return; }
    read({ fromEvent: true });
  };
  listen('chief-resources://changed', () => {
    if (!active()) return;
    if (busy) { refreshQueued = true; return; }
    read({ fromEvent: true });
  }).catch(() => {});
}
