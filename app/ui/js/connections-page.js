import { invoke } from './tauri.js';
import { pageRequest } from './page-host.js';
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const names = { new_api: 'New API', magpie: 'Magpie', cc_switch: 'CC Switch' };

export function pageConnections() {
  return `<section class="connections-space" data-connections-root>
    <div class="connections-toolbar"><p data-connection-status role="status" aria-live="polite">保存外部服务地址，不启动服务或修改工具配置。</p><button type="button" class="mini-btn" data-connection-refresh>刷新连接</button></div>
    <div data-connection-list></div>
    <details data-connection-editor><summary>添加连接</summary>
      <form data-connection-form class="sb-form">
        <input type="hidden" name="id">
        <label class="sb-field"><span>连接名称</span><input name="name" aria-label="连接名称" maxlength="80" required autocomplete="off"></label>
        <label class="sb-field"><span>服务类型</span><select name="kind" aria-label="服务类型"><option value="new_api">New API</option><option value="magpie">Magpie</option><option value="cc_switch">CC Switch</option></select></label>
        <label class="sb-field"><span>服务地址</span><input name="base_url" aria-label="服务地址" type="url" maxlength="2048" placeholder="http://127.0.0.1:端口/" required autocomplete="off"></label>
        <label class="sb-field"><span>凭据环境变量名 · 可选</span><input name="environment" aria-label="凭据环境变量名" maxlength="80" autocomplete="off" placeholder="填写变量名，不填写密钥"></label>
        <label class="connection-enabled"><input type="checkbox" name="enabled" checked>启用连接</label>
        <p class="model-scope">只保存变量名；凭据需在应用启动环境中提供。保存不访问服务，状态读取不验证模型或用量权限。</p>
        <div class="actions"><button type="submit" class="mini-btn" data-connection-save>保存连接</button><button type="button" class="mini-btn" data-connection-cancel>取消编辑</button></div>
      </form>
    </details>
    <div class="connection-delete" data-connection-delete hidden role="group" aria-label="删除连接确认"><p></p><button type="button" class="mini-btn" data-connection-delete-yes>删除连接</button><button type="button" class="mini-btn" data-connection-delete-no>取消删除</button></div>
  </section>`;
}

export async function hydrateConnections(root) {
  if (!root || root.dataset.pageReady === 'true') return;
  root.dataset.pageReady = 'true';
  const request = pageRequest(root);
  let snapshot = null, busy = false, removeId = null, hasDraft = false;
  const probes = new Map();
  const status = root.querySelector('[data-connection-status]');
  const list = root.querySelector('[data-connection-list]');
  const editor = root.querySelector('[data-connection-editor]');
  const form = root.querySelector('[data-connection-form]');
  const deletion = root.querySelector('[data-connection-delete]');
  const field = name => form.elements.namedItem(name);
  const lock = () => {
    root.querySelectorAll('button,input,select').forEach(control => { control.disabled = busy || (!snapshot && control.dataset.connectionRefresh === undefined); });
  };
  const feedback = text => { status.textContent = text; };
  form.addEventListener('input', () => { hasDraft = true; });
  form.addEventListener('change', () => { hasDraft = true; });
  function reset() {
    form.reset(); field('id').value = ''; editor.open = false;
    hasDraft = false;
    editor.querySelector('summary').textContent = '添加连接';
  }
  function rows() {
    const labels = { readable:'状态可读',disabled:'已停用',unsupported:'接口未核实',offline:'不可达',auth_failed:'鉴权失败',permission_denied:'无读取权限',error:'读取失败' };
    list.innerHTML = snapshot.items.length ? snapshot.items.map(item => {
      const probe = probes.get(item.id);
      const warning = item.kind === 'magpie' && item.credential_ref && item.base_url.startsWith('http:');
      return `<article class="connection-row"><div><strong>${esc(item.name)}</strong><span class="sb-tag" data-connection-state="${esc(item.enabled ? probe?.status ?? 'untested' : 'disabled')}">${item.enabled ? labels[probe?.status] ?? (item.kind === 'cc_switch' ? '接口未核实' : '未检测') : '已停用'}</span><p>${esc(names[item.kind])} · ${esc(item.base_url)}</p><small>${item.credential_ref ? `环境变量：${esc(item.credential_ref.name)}` : '未配置凭据'}</small>${probe ? `<p role="status">${esc(probe.reason)}${probe.service_version ? ` · ${esc(probe.service_version)}` : ''}<span class="connection-checked">上次读取 · ${esc(new Date(probe.checked_at_ms).toLocaleString([], { month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',second:'2-digit' }))}</span></p>` : ''}${warning ? '<p>HTTP 读取会以明文发送凭据。</p>' : ''}</div><div class="actions">${item.enabled && item.kind !== 'cc_switch' ? `<button type="button" class="mini-btn" data-connection-test="${esc(item.id)}">${item.kind === 'new_api' ? '读取公开状态' : warning ? '读取状态（HTTP）' : '读取状态'}</button>` : ''}<button type="button" class="mini-btn" data-connection-edit="${esc(item.id)}">编辑</button><button type="button" class="mini-btn" data-connection-remove="${esc(item.id)}">移除</button></div></article>`;
    }).join('') : '<div class="sb-empty">还没有外部连接。</div>';
  }
  async function run(action) {
    if (busy) return;
    busy = true; lock();
    try { await action(); }
    catch (error) { if (request.ownsRequest()) feedback(String(error)); }
    finally { busy = false; if (request.ownsRequest()) lock(); }
  }
  async function read() {
    feedback('正在读取连接');
    const next = await invoke('connections_list');
    if (!request.ownsRequest()) return;
    snapshot = next; probes.clear(); rows(); deletion.hidden = true; removeId = null;
    feedback('手动读取已保存服务的状态；不会启动服务或修改工具配置。');
  }
  root.querySelector('[data-connection-refresh]').onclick = () => run(read);
  root.querySelector('[data-connection-cancel]').onclick = reset;
  form.addEventListener('submit', event => {
    event.preventDefault();
    run(async () => {
      const config = { id: field('id').value || null, name: field('name').value, kind: field('kind').value,
        base_url: field('base_url').value, enabled: field('enabled').checked,
        credential_ref: field('environment').value.trim() ? { kind: 'environment', name: field('environment').value.trim() } : null };
      const next = await invoke('connection_save', { config, expectedRevision: snapshot.revision });
      if (!request.ownsRequest()) return;
      snapshot = next; probes.clear(); rows(); reset(); deletion.hidden = true; removeId = null; feedback('连接已保存，尚未检测。');
    });
  });
  list.addEventListener('click', event => {
    if (busy || !snapshot) return;
    const test = event.target.closest('[data-connection-test]');
    if (test) {
      run(async () => {
        probes.delete(test.dataset.connectionTest); rows(); lock();
        feedback('正在读取服务状态');
        const probe = await invoke('connection_test', { id: test.dataset.connectionTest, expectedRevision: snapshot.revision });
        if (!request.ownsRequest()) return;
        probes.set(probe.connection_id, probe); rows(); feedback(probe.reason);
      });
      return;
    }
    const edit = event.target.closest('[data-connection-edit]');
    const remove = event.target.closest('[data-connection-remove]');
    const item = snapshot.items.find(item => item.id === (edit?.dataset.connectionEdit ?? remove?.dataset.connectionRemove));
    if (!item) return;
    if (edit) {
      // Refuse to silently replace a draft from a different connection.
      if (hasDraft) { feedback('请先保存或取消当前编辑。'); return; }
      deletion.hidden = true; removeId = null;
      form.reset();
      for (const name of ['id','name','kind','base_url']) field(name).value = item[name];
      field('environment').value = item.credential_ref?.name ?? ''; field('enabled').checked = item.enabled;
      editor.querySelector('summary').textContent = '编辑连接'; editor.open = true; field('name').focus();
    } else {
      removeId = item.id; deletion.hidden = false;
      deletion.querySelector('p').textContent = `移除「${item.name}」的本地连接？服务本身不受影响。`;
      deletion.querySelector('[data-connection-delete-no]').focus();
    }
  });
  deletion.querySelector('[data-connection-delete-no]').onclick = () => { deletion.hidden = true; removeId = null; };
  deletion.querySelector('[data-connection-delete-yes]').onclick = () => run(async () => {
    if (!removeId) return;
    if (hasDraft) { feedback('请先保存或取消当前编辑。'); return; }
    const next = await invoke('connection_remove', { id: removeId, expectedRevision: snapshot.revision });
    if (!request.ownsRequest()) return;
    if (field('id').value === removeId) reset();
    snapshot = next; probes.clear(); rows(); deletion.hidden = true; removeId = null; feedback('本地连接已移除。');
  });
  await run(read);
}
