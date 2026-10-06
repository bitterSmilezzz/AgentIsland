// Configured targets only. A profile is not proof of model access or a running session.
const escape = value => String(value ?? '').replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');

export function configuredModels(status, profiles) {
  const rows = new Map();
  const add = (model, provider, protocol, endpoint, profile, current, profileId, credentialRef) => {
    if (typeof model !== 'string' || !model.trim()) return;
    const key = JSON.stringify([model, provider, protocol, endpoint, credentialRef ?? '']);
    const item = rows.get(key) ?? { model, provider, protocol, endpoint, credentialRef: credentialRef ?? '', profiles: [], choices: [], current: false };
    if (profile) item.profiles.push(profile);
    if (profileId) item.choices.push({ id: profileId, name: profile });
    item.current ||= current;
    rows.set(key, item);
  };
  for (const profile of profiles) add(profile.model, profile.provider_id ?? '默认接口', profile.wire_api ?? '', profile.base_url ?? '', profile.name, false, profile.id, profile.env_key);
  if (!status.config_error) {
    const active = status.current_draft ?? profiles.find(profile => profile.id === status.active_profile_id);
    add(status.configured_model, status.configured_provider ?? '默认接口', active?.wire_api ?? '', active?.base_url ?? '', null, true, null, active?.env_key);
  }
  return [...rows.values()].sort((a, b) => Number(b.current) - Number(a.current) || a.model.localeCompare(b.model));
}

export function modelDirectoryHtml(status, profiles, profilesAvailable = true, view = 'models', query = '') {
  const rows = configuredModels(status, profiles);
  return `<div class="model-directory" data-catalog-kind="${view === 'interfaces' ? 'interfaces' : 'models'}">
    <div class="catalog-controls"><div role="group" aria-label="目录视图"><button type="button" class="mini-btn" data-catalog-view="models" aria-pressed="${view !== 'interfaces'}">模型</button><button type="button" class="mini-btn" data-catalog-view="interfaces" aria-pressed="${view === 'interfaces'}">接口</button></div><input type="search" data-catalog-query aria-label="搜索模型与接口" placeholder="搜索模型、接口或档位" value="${escape(query)}"></div>
    <p class="model-scope" role="status" data-model-feedback hidden></p>
    ${!profilesAvailable || status.config_error ? '<p role="status" class="model-scope">部分配置读取失败，目录可能不完整。请回到工具配置刷新。</p>' : ''}
    <div data-catalog-panel="models"${view === 'interfaces' ? ' hidden' : ''}><p class="model-scope">选择本机 Codex 档位，预览后应用；运行中的会话保持当前模型。</p>${rows.length ? `<ul>${rows.map(row => `<li data-catalog-row data-catalog-search="${escape([row.model,row.provider,row.protocol,row.endpoint,row.credentialRef,...row.profiles].join(' '))}"><div><strong>${escape(row.model)}</strong>${row.current ? '<span class="sb-tag">配置目标</span>' : ''}<p>${escape(row.provider)}${row.protocol ? ` · ${escape(row.protocol)}${row.protocol === 'chat' ? '（旧档位，需编辑协议）' : ''}` : ''}</p>${row.credentialRef ? `<p>认证引用：${escape(row.credentialRef)}</p>` : ''}${row.endpoint ? `<p class="model-endpoint">${escape(row.endpoint)}</p>` : ''}</div><span class="model-origin">Codex${row.profiles.length ? `<small>${row.profiles.length} 个档位</small>` : ''}</span>${row.choices.length ? `<div class="model-choices">${row.choices.map(choice => `<button type="button" class="mini-btn" data-model-profile="${escape(choice.id)}"${status.config_error || !status.revision || !profilesAvailable || row.protocol !== 'responses' ? ' disabled' : ''}>预览 ${escape(choice.name)}</button>`).join('')}</div>` : ''}</li>`).join('')}</ul>` : `<div class="sb-empty">${!profilesAvailable || status.config_error ? '暂无可读取的模型。' : '尚未指定模型。可在工具配置中添加档位。'}</div>`}</div>
    <div data-catalog-panel="interfaces"${view === 'interfaces' ? '' : ' hidden'}>${interfaceDirectoryHtml(status, profiles, profilesAvailable)}</div>
    <p data-catalog-count role="status" aria-live="polite" class="model-scope"></p>
  </div>`;
}

// No URL normalization: different configured endpoints must remain distinguishable.
export function configuredInterfaces(status, profiles) {
  const grouped = new Map();
  for (const row of configuredModels(status, profiles)) {
    const key = JSON.stringify([row.provider, row.protocol, row.endpoint, row.credentialRef]);
    const item = grouped.get(key) ?? {provider:row.provider, protocol:row.protocol, endpoint:row.endpoint, credentialRef:row.credentialRef, current:false, models:[], choices:[]};
    item.current ||= row.current;
    item.models.push(row.model);
    item.choices.push(...row.choices.map(choice=>({...choice,model:row.model})));
    grouped.set(key,item);
  }
  return [...grouped.values()].sort((a,b)=>Number(b.current)-Number(a.current)||a.provider.localeCompare(b.provider)||a.endpoint.localeCompare(b.endpoint));
}
export function interfaceDirectoryHtml(status, profiles, available = true) {
  const rows=configuredInterfaces(status,profiles);
  return `<p class="model-scope">来自本机 Codex 配置与档位。认证只显示变量名；地址与协议不代表服务已验证。</p>${rows.length?`<ul>${rows.map(row=>`<li data-catalog-row data-catalog-search="${escape([row.provider,row.protocol,row.endpoint,row.credentialRef,...row.models,...row.choices.map(c=>c.name)].join(' '))}"><div><strong>${escape(row.provider)}</strong>${row.current?'<span class="sb-tag">配置目标</span>':''}<span class="catalog-model-count">${row.models.length} 个模型</span><p class="model-endpoint">${escape(row.endpoint || '工具默认地址')}</p><p>${escape(row.protocol || '协议未指定')} · 认证引用 ${escape(row.credentialRef || '未指定')}</p></div><div class="model-choices">${row.choices.map(c=>`<div class="catalog-choice"><span><strong>${escape(c.name)}</strong><small>${escape(c.model)}</small></span><button type="button" class="mini-btn" aria-label="预览应用 ${escape(c.name)}" data-model-profile="${escape(c.id)}"${status.config_error||!status.revision||!available||row.protocol!=='responses'?' disabled':''}>预览</button><button type="button" class="mini-btn" aria-label="编辑档位 ${escape(c.name)}" data-catalog-edit="${escape(c.id)}">编辑</button></div>`).join('')}</div></li>`).join('')}</ul>`:'<div class="sb-empty">尚无可读取的接口。可在工具配置中添加档位。</div>'}`;
}
export function bindModelDirectory(directory) {
  if(!directory)return;
  const filter=()=>{
    const value=directory.querySelector('[data-catalog-query]')?.value.trim().toLocaleLowerCase()??'';
    const panel=[...directory.querySelectorAll('[data-catalog-panel]')].find(p=>!p.hidden);
    if(!panel)return;
    const rows=[...panel.querySelectorAll('[data-catalog-row]')];
    let visible=0;for(const row of rows){row.hidden=!!value&&!row.dataset.catalogSearch.toLocaleLowerCase().includes(value);if(!row.hidden)visible++;}
    directory.querySelector('[data-catalog-count]').textContent=value?(visible?`找到 ${visible} 项`:'没有匹配的配置。'):'';
  };
  directory.oninput=filter;
  if(!directory.catalogBound){directory.catalogBound=true;directory.addEventListener('click',e=>{
    const button=e.target.closest('[data-catalog-view]');if(!button)return;
    const kind=button.dataset.catalogView;
    directory.querySelector('.model-directory').dataset.catalogKind=kind;
    for(const b of directory.querySelectorAll('[data-catalog-view]'))b.setAttribute('aria-pressed',String(b===button));
    for(const p of directory.querySelectorAll('[data-catalog-panel]'))p.hidden=p.dataset.catalogPanel!==kind;
    // Read fresh input/panels after the provider refresh replaces this subtree.
    directory.oninput?.();
  });
    directory.addEventListener('keydown', e => {
      const button = e.target.closest('[data-catalog-view]');
      if (!button || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
      const buttons = [...directory.querySelectorAll('[data-catalog-view]')];
      const index = buttons.indexOf(button);
      let next;
      if (e.key === 'ArrowRight') next = (index + 1) % buttons.length;
      else if (e.key === 'ArrowLeft') next = (index + buttons.length - 1) % buttons.length;
      else if (e.key === 'Home') next = 0;
      else if (e.key === 'End') next = buttons.length - 1;
      else return;
      e.preventDefault();
      buttons[next].focus();
      buttons[next].click();
    });
  }
  filter();
}

export function bindModelWorkspace(workspace, openServices, openExtensions, openPrompts) {
  if (!workspace || workspace.dataset.modelsBound === 'true') return;
  workspace.dataset.modelsBound = 'true';
  const controls = [...workspace.querySelectorAll('[data-model-view]')];
  controls.forEach(button => button.addEventListener('click', () => {
    if (workspace.querySelector('[data-confirm]:not([hidden]),[data-mcp-confirm]:not([hidden]),[data-prompt-confirm]:not([hidden]),[data-claude-confirm]:not([hidden])')) return;
    const selected = button.dataset.modelView;
    controls.forEach(control => control.setAttribute('aria-pressed', String(control === button)));
    workspace.querySelectorAll('[data-model-panel]').forEach(panel => { panel.hidden = panel.dataset.modelPanel !== selected; });
    if (selected === 'services') openServices?.();
    if (selected === 'extensions') openExtensions?.();
    if (selected === 'prompts') openPrompts?.();
  }));
}
