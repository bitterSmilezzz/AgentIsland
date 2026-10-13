import { invoke } from './tauri.js';
import { pageOwnsFocus } from './page-host.js';
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const identity = profile => JSON.stringify(Object.entries(profile).sort(([a],[b]) => a.localeCompare(b)));

// Page-local results only. A changed saved profile cannot inherit another target's result.
export function createModelReader(read, changed = () => {}) {
  let profiles = [], busy = null;
  const results = new Map();
  const keyFor = id => profiles.find(p => p.id === id);
  const api = {
    update(next) {
      profiles = next;
      for (const [key, state] of results) if (!profiles.some(p => p.id === state.profile.id && identity(p) === key)) results.delete(key);
      changed();
    },
    get busy() { return busy !== null; },
    state(id) { const profile = keyFor(id); return profile ? results.get(identity(profile)) : null; },
    async load(id) {
      const profile = keyFor(id);
      if (!profile || busy) return false;
      const key = identity(profile), previous = results.get(key);
      if (!results.has(key) && results.size >= 8) results.delete(results.keys().next().value);
      const state = {profile, result:previous?.result ?? null, error:'', busy:true};
      results.set(key, state); busy = key; changed();
      try {
        const result = await read(profile);
        if (!result || !Array.isArray(result.models) || !Number.isSafeInteger(result.total) || !Number.isSafeInteger(result.checked_at_ms)) throw Error('模型读取未完成');
        if (identity(keyFor(id) ?? {}) === key && results.get(key) === state) state.result = result;
      } catch (error) {
        if (identity(keyFor(id) ?? {}) === key && results.get(key) === state) {
          if (error?.code === 'target_invalidated') state.result = null;
          state.error = typeof error?.message === 'string' ? error.message : String(error);
        }
      } finally { state.busy = false; busy = null; changed(); }
      return true;
    },
  };
  return api;
}

export function interfaceModelsHtml(choice, available) {
  if (!choice) return '<p class="model-scope">保存档位后可手动读取服务模型。</p>';
  return `<section class="interface-models" data-interface-models="${esc(choice.id)}" aria-label="${esc(choice.name)} 的服务模型">
    <div class="interface-model-heading"><button type="button" class="mini-btn" data-read-models="${esc(choice.id)}"${available ? '' : ' disabled'}>读取服务模型</button><span>使用 ${esc(choice.name)} · 手动访问此接口</span></div>
    <p role="status" aria-live="polite" data-interface-model-status></p>
    <div data-interface-model-result></div></section>`;
}

export function bindInterfaceModels(root, profiles) {
  if (!root) return;
  const render = () => {
    const reader = root.modelReader;
    const lease = root.querySelector('[data-model-read-lease]');
    if (lease && lease.checked !== reader.busy) { lease.checked = reader.busy; lease.dispatchEvent(new Event('change', {bubbles:true})); }
    for (const box of root.querySelectorAll('[data-interface-models]')) {
      const state = reader.state(box.dataset.interfaceModels), button = box.querySelector('[data-read-models]');
      button.disabled = reader.busy || button.dataset.unavailable === 'true';
      button.textContent = state?.busy ? '读取中…' : state?.result ? '重新读取' : state?.error ? '重试读取' : '读取服务模型';
      box.setAttribute('aria-busy', String(!!state?.busy));
      const result = state?.result;
      box.querySelector('[data-interface-model-status]').textContent = [state?.busy ? '正在读取，不发起生成请求。' : state?.error || '',result ? `${state?.error ? '上次成功：' : '读取于 '}${new Date(result.checked_at_ms).toLocaleString('zh-CN')} · ${result.total} 个模型${result.truncated ? '，显示前200个' : ''}` : ''].filter(Boolean).join(' ');
      const body = box.querySelector('[data-interface-model-result]');
      if (body.modelResult === result) continue;
      body.modelResult = result;
      body.innerHTML = result ? `<p>服务返回的清单，不代表生成权限、运行模型或账单。</p>${result.models.length ? `<ul class="interface-model-list" tabindex="0" aria-label="服务返回的模型">${result.models.map(model => `<li data-service-model="${esc(model)}"><span>${esc(model)}</span><button type="button" class="mini-btn" data-discovered-model="${esc(model)}" data-catalog-edit="${esc(box.dataset.interfaceModels)}" aria-label="将 ${esc(model)} 填入 ${esc(state.profile.name)} 档位草稿">填入档位</button></li>`).join('')}</ul>` : '<p>服务返回空模型列表。</p>'}` : '';
    }
    root.oninput?.();
  };
  if (!root.modelReader) {
    root.modelReader = createModelReader(profile => invoke('provider_read_models', {id:profile.id,expectedProfile:profile}), render);
    root.addEventListener('click', async event => {
      const button = event.target.closest('[data-read-models]');
      if (!button || button.disabled) return;
      const owned = pageOwnsFocus(root);
      await root.modelReader.load(button.dataset.readModels);
      if (owned && root.isConnected && !root.closest('[hidden]') && document.activeElement === document.body && button.isConnected && !button.disabled) button.focus({preventScroll:true});
    });
  }
  for (const button of root.querySelectorAll('[data-read-models]')) button.dataset.unavailable ??= String(button.disabled);
  root.modelReader.update(profiles);
}
