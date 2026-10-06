import { invoke } from './tauri.js';
import { pageRequest } from './page-host.js';
import {skillPackagesHtml,bindSkillPackages} from './skills-package-page.js';
const esc = value => String(value ?? '').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
export function pageMcp() {
  return `<section data-mcp-root class="mcp-space">
    <div class="connections-toolbar"><p data-mcp-status role="status" aria-live="polite">读取本机配置，不启动 MCP 服务器。</p><button class="mini-btn" type="button" data-mcp-refresh>刷新扩展</button></div>
    <div data-mcp-list></div>
    <details data-mcp-editor><summary>添加 MCP</summary><form class="sb-form" data-mcp-form>
      <label class="sb-field"><span>名称</span><input name="name" aria-label="MCP 名称" required maxlength="80" autocomplete="off"></label>
      <label class="sb-field"><span>连接方式</span><select name="kind" aria-label="MCP 连接方式"><option value="http">HTTP</option><option value="stdio">本地进程</option></select></label>
      <fieldset data-mcp-http><legend>HTTP 配置</legend>
        <label class="sb-field"><span>服务地址</span><input name="url" aria-label="MCP 服务地址" type="url" maxlength="2048" required autocomplete="off" placeholder="https://example.com/mcp"></label>
        <label class="sb-field"><span>认证环境变量名 · 可选</span><input name="bearer" aria-label="MCP 认证环境变量名" maxlength="80" autocomplete="off"></label>
      </fieldset>
      <fieldset data-mcp-stdio hidden><legend>本地进程配置</legend>
        <label class="sb-field"><span>启动程序</span><input name="command" aria-label="MCP 启动程序" maxlength="2048" required autocomplete="off" placeholder="可执行程序或路径"></label>
        <label class="sb-field"><span>参数 · JSON 数组</span><textarea name="args" aria-label="MCP 参数" maxlength="131072" rows="3">[]</textarea></label>
        <label class="sb-field"><span>环境变量名 · 每行一个</span><textarea name="environment" aria-label="MCP 环境变量名" maxlength="5184" rows="2"></textarea></label>
      </fieldset>
      <label class="connection-enabled"><input name="enabled" type="checkbox" checked>启用</label>
      <p class="model-scope">只填写凭据变量名。配置写入前先预览和备份；运行中的客户端需要重新读取配置。</p>
      <div class="actions"><button class="mini-btn" type="submit">预览配置</button><button class="mini-btn" type="button" data-mcp-cancel>取消编辑</button></div>
    </form></details>
    <div class="sb-confirm mcp-confirm" data-mcp-confirm hidden role="group" aria-label="MCP 配置预览"><div data-mcp-diff></div><p data-mcp-error role="alert"></p><div class="actions"><button class="mini-btn" type="button" data-mcp-apply>确认写入</button><button class="mini-btn" type="button" data-mcp-dismiss>取消预览</button></div></div>
    <details class="mcp-skills"><summary>本机 Skills</summary><p class="model-scope">Codex 用户级启停配置；标准技能包可安装与同步到所选工具。旧目录清单只读。</p><button type="button" class="mini-btn" data-skills-refresh>刷新 Skills</button><div data-mcp-skills>尚未读取</div>${skillPackagesHtml()}<details><summary>其他本机清单 · 只读</summary><div data-skills-legacy>尚未读取</div></details></details>
  </section>`;
}
const transportText = draft => draft ? draft.transport.kind === 'http' ? `HTTP · ${draft.transport.url}` : `本地进程 · ${draft.transport.command}` : '无配置';
export async function hydrateMcp(root) {
  if (!root || root.dataset.pageReady === 'true') return;
  root.dataset.pageReady = 'true';
  bindSkillPackages(root.querySelector('[data-skills-packages]'));
  const request = pageRequest(root), form = root.querySelector('[data-mcp-form]'), editor = root.querySelector('[data-mcp-editor]');
  const status = root.querySelector('[data-mcp-status]'), list = root.querySelector('[data-mcp-list]'), confirm = root.querySelector('[data-mcp-confirm]');
  const field = name => form.elements.namedItem(name);
  let snapshot=null,mcpFresh=false,busy=false,editing=false,dirty=false,pending=null,returnFocus=null,skills=null,skillsFresh=false,skillsLoaded=false,focusAfter=null,skillsRefreshQueued=false;
  const feedback = text => { status.textContent=text; };
  function lock() {
    root.querySelectorAll('button,input,select,textarea').forEach(control=>{if(control.closest('[data-skills-packages]'))return; control.disabled=busy || ((!mcpFresh || !snapshot?.writable) && !control.matches('[data-mcp-refresh],[data-skills-refresh]')); });
    const http=field('kind').value==='http';
    for (const [selector,active] of [['[data-mcp-http]',http],['[data-mcp-stdio]',!http]]) {
      const group=root.querySelector(selector);group.hidden=!active;group.disabled=!active||busy||!mcpFresh||!snapshot?.writable;
    }
    root.querySelectorAll('[data-skill-toggle]').forEach(control=>{control.disabled=busy||!skillsFresh||!skills?.writable;});
    field('name').readOnly=editing;
  }
  function reset() { form.reset();editing=false;dirty=false;editor.open=false;editor.querySelector('summary').textContent='添加 MCP';lock(); }
  function dismiss() {
    confirm.hidden=true;pending=null;
    for (const child of root.children) child.inert=false;
    const nav=root.closest('[data-model-workspace]')?.querySelector('.model-navigation');if(nav)nav.inert=false;
    if(returnFocus?.isConnected && returnFocus.getClientRects().length) returnFocus.focus();
  }
  function rows() {
    list.innerHTML=snapshot.entries.length?snapshot.entries.map((entry,index)=>`<article class="connection-row"><div><strong>${esc(entry.name)}</strong><span class="mcp-state">${entry.enabled===true?'配置启用':entry.enabled===false?'配置停用':'状态未知'}</span><p>${esc(entry.draft?transportText(entry.draft):entry.reason)}</p></div><div class="actions">${entry.draft?`<button class="mini-btn" type="button" data-mcp-edit="${index}">编辑</button>`:''}${entry.enabled!==null?`<button class="mini-btn" type="button" data-mcp-toggle="${index}">${entry.enabled?'停用':'启用'}</button>`:''}<button class="mini-btn" type="button" data-mcp-remove="${index}">删除</button></div></article>`).join(''):'<div class="sb-empty">还没有用户级 MCP 配置。</div>';
  }
  async function run(action) {
    if(busy)return;const caller=root.contains(document.activeElement)?document.activeElement:null;busy=true;root.setAttribute('aria-busy','true');lock();
    try { await action(); } catch(error) { if(request.ownsRequest()){ feedback(String(error));if(!confirm.hidden)confirm.querySelector('[data-mcp-error]').textContent=String(error); } }
    finally { busy=false;if(request.ownsRequest()){root.setAttribute('aria-busy','false');lock();if(!confirm.hidden && root.isConnected)confirm.querySelector('[data-mcp-dismiss]').focus();else if(root.isConnected && (!document.activeElement || document.activeElement===document.body || root.contains(document.activeElement))){let target=focusAfter??caller;if(focusAfter&&target.disabled)target=root.querySelector('[data-skills-refresh]');if(target?.isConnected&&!target.disabled&&!target.closest('[inert]')&&target.getClientRects().length)target.focus();}focusAfter=null;if(skillsRefreshQueued){skillsRefreshQueued=false;run(readSkills);}} }
  }
  async function read() {
    feedback('正在读取 MCP 配置');
    mcpFresh=false;
    let next;
    try { next=await invoke('mcp_inspect'); }
    catch { throw snapshot?'MCP 刷新失败，保留上次清单。请刷新后再修改。':'MCP 读取失败，请刷新重试。'; }
    if(!request.ownsRequest())return;
    snapshot=next;mcpFresh=true;rows();feedback(snapshot.notice);
  }
  async function preview(operation) {
    const plan=await invoke('mcp_preview',{operation,revision:snapshot.revision});
    if(!request.ownsRequest())return;
    confirm.setAttribute('aria-label','MCP 配置预览');pending={operation,plan};returnFocus=document.activeElement;confirm.hidden=false;confirm.querySelector('[data-mcp-error]').textContent='';
    root.querySelector('[data-mcp-diff]').innerHTML=`<strong>${esc(plan.action)} · ${esc(plan.name)}</strong><p>${esc(transportText(plan.before))} → ${esc(transportText(plan.after))}</p>${plan.after?`<p>目标状态：${plan.after.enabled?'启用':'停用'}</p>`:''}<details class="mcp-field-diff"><summary>查看字段差异</summary><p>当前配置</p><pre>${esc(JSON.stringify(plan.before,null,2))}</pre><p>目标配置</p><pre>${esc(JSON.stringify(plan.after,null,2))}</pre></details>${plan.preserves_advanced?'<p>保留工具策略、超时及其他非目标字段。</p>':''}<p>仅修改 Codex 用户级配置，写入前保存备份。不会立即重启客户端或验证连接。</p>`;
    for(const child of root.children) child.inert=child!==confirm;
    const nav=root.closest('[data-model-workspace]')?.querySelector('.model-navigation');if(nav)nav.inert=true;
    confirm.querySelector('[data-mcp-dismiss]').focus();
    confirm.scrollIntoView({block:'nearest'});
  }
  root.querySelector('[data-mcp-refresh]').onclick=()=>run(async()=>{await read();if(skillsLoaded)await readSkills();});
  form.addEventListener('input',()=>{dirty=true;});
  form.addEventListener('change',()=>{dirty=true;lock();});
  root.querySelector('[data-mcp-cancel]').onclick=reset;
  root.querySelector('[data-mcp-dismiss]').onclick=dismiss;
  confirm.onkeydown=event=>{
    if(event.key==='Escape'&&!busy){event.preventDefault();dismiss();}
    if(event.key==='Tab'){
      const controls=[...confirm.querySelectorAll('button,summary,[tabindex]')].filter(control=>!control.disabled&&control.tabIndex>=0&&control.getClientRects().length);
      if(!controls.length)return;
      event.preventDefault();const index=controls.indexOf(document.activeElement);
      controls[index<0?(event.shiftKey?controls.length-1:0):(index+(event.shiftKey?-1:1)+controls.length)%controls.length].focus();
    }
  };
  root.querySelector('[data-mcp-apply]').onclick=()=>run(async()=>{
    if(!pending)return;
    const {operation,plan}=pending;
    const result=await invoke(pending.skill?'skills_apply':'mcp_apply',{operation,revision:plan.revision,planId:plan.plan_id});
    if(!request.ownsRequest())return;
    const wasSkill=pending.skill;dismiss();if(!wasSkill)reset();mcpFresh=false;skillsFresh=false;
    let reloadNotice='';
    try { await read();if(skillsLoaded)await readSkills(); } catch { reloadNotice=' 配置已写入，但清单刷新失败；请刷新后再修改。'; }
    focusAfter=root.querySelector(wasSkill?`[data-skill-id="${operation.id}"]`:'[data-mcp-refresh]')??root.querySelector('[data-skills-refresh]');
    feedback(`${result.notice} 备份：${result.backup_name}，可在工具配置的备份与还原中刷新查看。${reloadNotice}`);
  });
  form.onsubmit=event=>{event.preventDefault();run(()=>{
    const lines=name=>field(name).value.split(/\r?\n/).map(text=>text.trim()).filter(Boolean);
    let args=[];if(field('kind').value==='stdio'){try{args=JSON.parse(field('args').value||'[]');}catch{throw '参数须为字符串 JSON 数组。';}if(!Array.isArray(args)||args.length>64||args.some(arg=>typeof arg!=='string'))throw '参数须为不超过 64 项的字符串 JSON 数组。';}
    const transport=field('kind').value==='http'?{kind:'http',url:field('url').value,bearer_token_env_var:field('bearer').value.trim()||null}:{kind:'stdio',command:field('command').value,args,env_vars:lines('environment')};
    return preview({kind:'save',replace:editing,config:{name:field('name').value,enabled:field('enabled').checked,transport}});
  });};
  list.onclick=event=>{
    if(busy||!mcpFresh||!snapshot?.writable||!confirm.hidden)return;
    const edit=event.target.closest('[data-mcp-edit]'),toggle=event.target.closest('[data-mcp-toggle]'),remove=event.target.closest('[data-mcp-remove]');
    const control=edit??toggle??remove;if(!control)return;
    if(dirty||editor.open){feedback('请先保存或取消当前编辑。');return;}
    const entry=snapshot.entries[Number(edit?.dataset.mcpEdit??toggle?.dataset.mcpToggle??remove?.dataset.mcpRemove)];if(!entry)return;
    if(edit&&entry.draft){
      form.reset();editing=true;dirty=true;editor.open=true;editor.querySelector('summary').textContent='编辑 MCP';
      const draft=entry.draft;field('name').value=draft.name;field('kind').value=draft.transport.kind;field('enabled').checked=draft.enabled;
      if(draft.transport.kind==='http'){field('url').value=draft.transport.url;field('bearer').value=draft.transport.bearer_token_env_var??'';}
      else{field('command').value=draft.transport.command;field('args').value=JSON.stringify(draft.transport.args);field('environment').value=draft.transport.env_vars.join('\n');}
      lock();field('kind').focus();
    }else run(()=>preview(toggle?{kind:'set_enabled',name:entry.name,enabled:!entry.enabled}:{kind:'remove',name:entry.name}));
  };
  async function readSkills() {
    const node=root.querySelector('[data-mcp-skills]');if(!skills)node.textContent='读取本机 Skills';feedback('正在读取 Skills 配置');skillsFresh=false;node.setAttribute('aria-busy','true');
    try {
      const next=await invoke('skills_inspect');if(!request.ownsRequest())return;
      skills=next;skillsFresh=true;skillsLoaded=true;feedback(next.notice);
      node.innerHTML=`<p class="model-scope">${esc(next.notice)}</p>${next.entries.length?next.entries.map((entry,index)=>`<article class="connection-row"><div><strong>${esc(entry.name)}</strong><span class="mcp-state">${entry.enabled?'配置启用':'配置停用'}</span><p>${entry.linked?'链接目录 · 按目标文件配置':'用户目录'}</p></div><button type="button" class="mini-btn" data-skill-toggle="${index}" data-skill-id="${esc(entry.id)}">${entry.enabled?'停用':'启用'}</button></article>`).join(''):'<p>用户目录内没有可管理的 Skills。</p>'}`;
    } catch {if(!request.ownsRequest())return;skillsLoaded=!!skills;const message=skills?'Skills 刷新失败，保留上次清单。请刷新后再修改。':'Skills 读取失败，请刷新重试。';if(!skills)node.textContent=message;throw message;}
    finally {if(request.ownsRequest())node.setAttribute('aria-busy','false');}
  }
  root.querySelector('[data-skills-refresh]').onclick=()=>run(readSkills);
  root.addEventListener('skill-package-changed',()=>{skillsFresh=false;lock();if(skillsLoaded){if(busy)skillsRefreshQueued=true;else run(readSkills);}});
  root.querySelector('.mcp-skills').addEventListener('toggle',event=>{if(event.target===event.currentTarget&&event.target.open&&!skillsLoaded)run(readSkills);});
  root.querySelector('[data-mcp-skills]').onclick=event=>{
    const button=event.target.closest('[data-skill-toggle]');if(!button||busy||!skillsFresh||!skills?.writable||!confirm.hidden)return;
    if(dirty||editor.open){feedback('请先保存或取消 MCP 编辑。');return;}
    const entry=skills.entries[Number(button.dataset.skillToggle)];if(!entry)return;
    run(async()=>{
      const operation={id:entry.id,enabled:!entry.enabled};const plan=await invoke('skills_preview',{operation,revision:skills.revision});if(!request.ownsRequest())return;
      pending={operation,plan,skill:true};returnFocus=button;confirm.hidden=false;confirm.setAttribute('aria-label','Skill 启停预览');confirm.querySelector('[data-mcp-error]').textContent='';
      root.querySelector('[data-mcp-diff]').innerHTML=`<strong>${esc(plan.name)}</strong><p>配置${plan.before?'启用':'停用'} → 配置${plan.after?'启用':'停用'}</p>${plan.linked?'<p>链接目录按目标 SKILL.md 配置；共享该目标的入口会使用同一配置。</p>':''}<p>仅更新 Codex 用户配置并备份；技能文件不改动，不执行脚本。请重启 Codex 核对加载。</p>`;
      for(const child of root.children)child.inert=child!==confirm;
      const nav=root.closest('[data-model-workspace]')?.querySelector('.model-navigation');if(nav)nav.inert=true;
      confirm.scrollIntoView({block:'nearest'});
    });
  };
  root.querySelector('[data-skills-legacy]').parentElement.addEventListener('toggle',async event=>{
    if(!event.target.open)return;const node=root.querySelector('[data-skills-legacy]');node.textContent='读取本机清单';
    try{const result=await invoke('provider_capabilities');if(!request.ownsRequest())return;node.innerHTML=result.items.filter(item=>item.kind==='Skill'&&item.source!=='~/.agents/skills').map(item=>`<p><strong>${esc(item.name)}</strong> · ${esc(item.target)}<br><small>${esc(item.status)}</small></p>`).join('')||'<p>其他检查目录内没有 Skills。</p>';}
    catch{if(request.ownsRequest())node.textContent='清单读取失败，请重新展开。';}
  });
  await run(read);
}
