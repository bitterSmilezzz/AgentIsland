import {invoke} from './tauri.js';
import {pageRequest} from './page-host.js';
const esc=value=>String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const toolName={codex:'Codex',claude:'Claude Code'};
const targetLabel=value=>toolName[value]??'目标未核实';
const actionName={install:'安装',update:'更新',restore:'恢复旧包',remove:'移回本次安装',trash:'移入废纸篓',edit:'保存正文'};
const operationName=preview=>preview.source?(preview.action==='update'?'同步更新':'同步'):(actionName[preview.action]??'技能操作');
const kindName={added:'新增',changed:'变化',removed:'移除',moved:'移出'};
export const packageSize=bytes=>bytes<1024?`${bytes} B`:bytes<1048576?`${(bytes/1024).toFixed(1)} KiB`:`${(bytes/1048576).toFixed(2)} MiB`;
export function skillPackagePreviewHtml(preview){
  return `<strong>${esc(operationName(preview))} · ${esc(preview.name)}</strong><p>${preview.action==='trash'?'恢复记录':`${targetLabel(preview.target??'codex')} 用户级`} · ${Number(preview.files)||0} 个${preview.action==='trash'?'移出':'目标'}文件 · ${packageSize(Number(preview.bytes)||0)}</p><p>${esc(preview.notice)}</p><details><summary>文件差异 · ${Number(preview.change_count)||0} 项</summary><ul class="skill-package-changes">${preview.changes.map(change=>`<li><span>${esc(kindName[change.kind]??'变化')}</span><code>${esc(change.path)}</code></li>`).join('')}</ul>${preview.change_count>preview.changes.length?`<p>显示前 ${preview.changes.length} 项；操作会核对完整目录。</p>`:''}</details>`;
}
export function skillPackageRecordsHtml(rows){
  if(!rows.length)return '<p class="model-scope">还没有本应用的安装记录。</p>';
  const rowHtml=(row,index)=>`<article class="connection-row"><div><strong>${esc(row.name)}</strong><span class="mcp-state">${row.target?`${esc(targetLabel(row.target))} · `:''}${esc({applied:'目录已安装',inactive:'本次包未应用',conflict:'文件已变化',unreadable:'记录不可读'}[row.state]??'待核对')}</span><p>${esc(row.notice)}</p>${row.created_ms>0?`<small>${esc(new Date(row.created_ms).toLocaleString('zh-CN'))}</small>`:''}</div><div class="actions">${row.state==='applied'?`<button type="button" class="mini-btn" data-skill-package-restore="${index}">恢复预览</button>`:''}${row.id?`<button type="button" class="mini-btn" data-skill-package-trash="${index}" aria-label="${esc(row.name)} · 移入废纸篓">移入废纸篓</button>`:''}</div></article>`;
  return rows.slice(0,5).map(rowHtml).join('')+(rows.length>5?`<details class="skill-package-history"><summary>更多安装记录 · ${rows.length-5} 条</summary>${rows.slice(5).map((row,index)=>rowHtml(row,index+5)).join('')}</details>`:'');
}
export function skillSyncRowsHtml(entries){
  const render=(row,index)=>`<article class="connection-row"><div><strong>${esc(targetLabel(row.tool))} · ${esc(row.name)}</strong><p>${esc(row.notice)}</p></div><div class="actions"><button class="mini-btn" type="button" data-skill-body-read="${index}" aria-label="${esc(targetLabel(row.tool))} · ${esc(row.name)} · 编辑正文">编辑正文</button><button class="mini-btn" type="button" data-skill-sync-preview="${index}">${row.available?'同步预览':'不可预览'}</button></div></article>`;
  if(!entries.length)return '<p class="model-scope">没有可列出的用户级技能目录。</p>';
  return entries.slice(0,5).map(render).join('')+(entries.length>5?`<details><summary>更多来源 · ${entries.length-5} 项</summary>${entries.slice(5).map((row,i)=>render(row,i+5)).join('')}</details>`:'');
}
export function skillPackagesHtml(){return `<details data-skills-packages class="skill-packages"><summary>安装与恢复</summary>
  <p class="model-scope">标准技能包可安装至所选工具的用户目录，更新保留旧包。加载与工具专属语法需在目标客户端核对。</p>
  <div class="actions"><label class="skill-package-target">目标工具<select data-skill-package-target aria-label="技能目标工具" disabled><option value="codex">Codex</option><option value="claude">Claude Code</option></select></label><button type="button" class="mini-btn" data-skill-package-choose disabled>选择技能目录</button><button type="button" class="mini-btn" data-skill-package-refresh disabled>刷新安装记录</button></div>
  <p data-skill-package-status role="status" aria-live="polite">展开后读取安装记录。</p>
  <details data-skill-sync><summary>已有技能 · 编辑与同步</summary><p class="model-scope">编辑原目录正文；同步到上方所选工具。保存前核对差异。</p><button type="button" class="mini-btn" data-skill-sync-refresh disabled>刷新来源</button><p data-skill-sync-status role="status" aria-live="polite">展开后读取来源目录。</p><div data-skill-sync-list></div></details>
  <section data-skill-body-editor hidden class="skill-body-editor" role="group" aria-label="技能正文编辑">
    <strong data-skill-body-title></strong><p class="model-scope">仅编辑正文，元数据与包内资源保留；保存前预览，旧版本可恢复。最多4 MiB，请使用凭据引用。</p>
    <label class="sb-field"><span>正文 · Markdown</span><textarea data-skill-body-text aria-label="技能正文" rows="12" maxlength="4194304" spellcheck="false"></textarea></label>
    <p data-skill-body-status role="status" aria-live="polite"></p><p data-skill-body-limit class="model-scope" role="status" hidden>安装记录已满。请先关闭编辑，再移出不再需要的记录。</p><div class="actions"><button type="button" class="mini-btn" data-skill-body-preview>预览保存</button><button type="button" class="mini-btn" data-skill-body-close>关闭编辑</button></div>
  </section>
  <div data-skill-package-records></div>
  <section class="sb-confirm" data-confirm data-skill-package-confirm hidden role="group" aria-label="技能包操作预览"><div data-skill-package-diff></div><p data-skill-package-error role="alert"></p><div class="actions"><button type="button" class="mini-btn" data-skill-package-apply>确认安装</button><button type="button" class="mini-btn" data-skill-package-cancel>取消预览</button></div></section>
</details>`;}
export function bindSkillPackages(root){
  if(!root||root.dataset.bound==='true')return;root.dataset.bound='true';
  const request=pageRequest(root),status=root.querySelector('[data-skill-package-status]'),records=root.querySelector('[data-skill-package-records]'),confirm=root.querySelector('[data-skill-package-confirm]');
  const choose=root.querySelector('[data-skill-package-choose]'),refresh=root.querySelector('[data-skill-package-refresh]'),apply=root.querySelector('[data-skill-package-apply]'),cancel=root.querySelector('[data-skill-package-cancel]'),error=root.querySelector('[data-skill-package-error]');
  const target=root.querySelector('[data-skill-package-target]'),sync=root.querySelector('[data-skill-sync]'),syncRefresh=root.querySelector('[data-skill-sync-refresh]'),syncStatus=root.querySelector('[data-skill-sync-status]'),syncList=root.querySelector('[data-skill-sync-list]');
  const editor=root.querySelector('[data-skill-body-editor]'),bodyText=root.querySelector('[data-skill-body-text]'),bodyPreview=root.querySelector('[data-skill-body-preview]'),bodyClose=root.querySelector('[data-skill-body-close]'),bodyStatus=root.querySelector('[data-skill-body-status]'),bodyLimit=root.querySelector('[data-skill-body-limit]');
  const toolbar=choose.closest('.actions'),intro=root.querySelector(':scope > .model-scope');
  let editing=null;
  let syncStarted=false,syncFresh=false,inventory=null;
  let started=false,supported=null,busy=false,fresh=false,rows=[],pending=null,caller=null,timer=null,focusAfter=null;const inert=new Map();
  const feedback=text=>{status.textContent=text;};
  const expired=()=>pending&&!pending.id&&Date.now()>pending.preview.expires_ms;
  function lock(){
    for(const section of [toolbar,intro,status,sync,records])section.hidden=!!editing;
    const dirty=!!editing&&bodyText.value!==editing.body;
    const atCapacity=fresh&&rows.length>=100;
    bodyText.disabled=busy||!!pending;bodyText.readOnly=!!editing?.attempted;
    bodyPreview.disabled=busy||!!pending||!dirty||atCapacity||!!editing?.attempted;
    bodyLimit.hidden=!editing||!atCapacity;
    bodyClose.disabled=busy||!!pending;
    bodyClose.textContent=editing?.attempted?'关闭编辑':dirty?'放弃草稿':'关闭编辑';
    cancel.textContent=pending?.attempted?'关闭预览':'取消预览';
    root.querySelectorAll('[data-skill-body-read]').forEach(button=>{const row=inventory?.entries[Number(button.dataset.skillBodyRead)];button.disabled=busy||supported!==true||!!pending||!!editing||!syncFresh||!row?.available;});
    target.disabled=busy||supported!==true||!!pending||!!editing;syncRefresh.disabled=busy||supported!==true||!!pending||!!editing;root.querySelectorAll('[data-skill-sync-preview]').forEach(button=>{const row=inventory?.entries[Number(button.dataset.skillSyncPreview)];button.disabled=busy||supported!==true||!!pending||!!editing||!syncFresh||!row?.available||row.tool===target.value||(fresh&&rows.length>=100);button.title=row?.tool===target.value?'来源与目标相同':'';});choose.disabled=busy||supported!==true||!!pending||!!editing||(fresh&&rows.length>=100);refresh.disabled=busy||supported===false||!!pending||!!editing;apply.disabled=busy||!pending||expired()||!!pending?.attempted;cancel.disabled=busy;root.querySelectorAll('[data-skill-package-restore],[data-skill-package-trash]').forEach(button=>button.disabled=busy||!fresh||!!pending||!!editing);root.setAttribute('aria-busy',String(busy));}
  function releaseInert(){for(const [node,value] of inert)node.inert=value;inert.clear();}
  function constrain(panel=confirm){
    releaseInert();
    for(let node=panel;node&&node!==root.closest('[data-mcp-root]');node=node.parentElement){for(const sibling of node.parentElement?.children??[]){if(sibling!==node&&!inert.has(sibling)){inert.set(sibling,sibling.inert);sibling.inert=true;}}}
    const nav=root.closest('[data-model-workspace]')?.querySelector('.model-navigation');if(nav&&!inert.has(nav)){inert.set(nav,nav.inert);nav.inert=true;}
  }
  function show(preview,id=null,origin=choose){
    caller=origin;pending={preview,id};confirm.hidden=false;error.textContent='';if(preview.action==='edit'){editor.hidden=true;records.hidden=true;}
    confirm.querySelector('[data-skill-package-diff]').innerHTML=skillPackagePreviewHtml(preview);apply.textContent=`确认${operationName(preview)}`;
    constrain();
    if(!id){clearTimeout(timer);timer=setTimeout(()=>{if(pending&&!pending.id){error.textContent='预览已到期，请取消后重新预览。';lock();}},Math.max(0,Math.min(300001,preview.expires_ms-Date.now()+1)));}
    if(root.isConnected){cancel.focus();confirm.scrollIntoView({block:'nearest'});}
  }
  function dismiss(){clearTimeout(timer);pending=null;confirm.hidden=true;records.hidden=false;if(editing)editor.hidden=false;releaseInert();}
  async function run(action){
    if(busy)return;const origin=root.contains(document.activeElement)?document.activeElement:null;busy=true;lock();
    try{await action();}catch(failure){if(request.ownsRequest()){feedback(String(failure));if(!confirm.hidden)error.textContent=String(failure);if(editing)bodyStatus.textContent=String(failure);}}
    finally{busy=false;if(request.ownsRequest()){lock();if(root.isConnected){if(pending)cancel.focus();else if(focusAfter||document.activeElement===document.body||document.activeElement===origin){const target=focusAfter??origin;if(target?.isConnected&&!target.disabled&&!target.closest('[inert]')&&target.getClientRects().length){if(target===bodyText)editor.scrollIntoView({block:'nearest'});target.focus({preventScroll:target===bodyText});}}}focusAfter=null;}}
  }
  async function read(){fresh=false;feedback('正在核对安装记录');let next;
    try{next=await invoke('skills_package_recoveries');}catch{throw rows.length?'安装记录刷新失败，保留上次清单。请刷新后再操作。':'安装记录读取失败，请刷新重试。';}
    if(!request.ownsRequest())return;rows=next;fresh=true;
    records.innerHTML=skillPackageRecordsHtml(rows);
    feedback(rows.length?`已核对 ${rows.length} 条安装记录。${rows.length>=100?' 名额已满，可核对并移出不再需要的记录。':''}`:'可选择本地目录安装。');
  }
  async function load(){
    if(supported===null){
      feedback('正在读取安装能力');
      const next=await invoke('skills_package_capability');
      if(!request.ownsRequest())return;supported=next;
    }
    if(!supported){
      feedback('本平台的技能包安装尚未验证。');
      if(sync.open)syncStatus.textContent='本平台的技能编辑与同步尚未验证。';
      return;
    }
    await read();
    if(syncStarted||sync.open)await readSync();
  }
  target.onchange=()=>lock();
  async function readSync(){
    syncStarted=true;syncFresh=false;syncStatus.textContent='正在核对来源目录';
    let next;try{next=await invoke('skills_package_inventory');}catch{if(request.ownsRequest())syncStatus.textContent='来源刷新失败，保留上次清单。请刷新后再同步。';return;}
    if(!request.ownsRequest())return;const unavailable=next.unavailable??[],retained=inventory?.entries.filter(row=>unavailable.includes(row.tool)).map(row=>({...row,id:'',available:false,notice:'上次清单，来源目录未核实；请刷新重试。'}))??[];inventory={...next,entries:[...next.entries,...retained]};syncFresh=true;syncList.innerHTML=skillSyncRowsHtml(inventory.entries);syncStatus.textContent=next.notices.length?next.notices.join('；'):`已读取 ${next.entries.length} 个目录候选，预览时核对完整包。`;
  }
  sync.addEventListener('toggle',event=>{
    if(event.target!==sync||!sync.open||syncStarted)return;
    if(supported!==true){
      syncStatus.textContent=supported===false?'本平台的技能编辑与同步尚未验证。':'正在读取安装能力';
      return;
    }
    if(busy){syncStatus.textContent='正在核对安装记录，随后读取来源';return;}
    run(readSync);
  });
  syncRefresh.onclick=()=>run(readSync);
  syncList.onclick=event=>{
    const editButton=event.target.closest('[data-skill-body-read]');
    if(editButton){if(busy||pending||editing||!syncFresh)return;const row=inventory?.entries[Number(editButton.dataset.skillBodyRead)];if(!row?.available)return;
      run(async()=>{constrain(editor);try{const document=await invoke('skills_package_edit_read',{id:row.id,generation:inventory.generation});if(!request.ownsRequest())return;editing={...document,origin:editButton};editor.hidden=false;bodyText.value=document.body;editing.body=bodyText.value;root.querySelector('[data-skill-body-title]').textContent=`${targetLabel(document.tool)} · ${document.name}`;bodyStatus.textContent='草稿保留在当前窗口；关闭应用前请保存或自行保留。';constrain(editor);focusAfter=bodyText;}catch(failure){if(request.ownsRequest()){syncFresh=false;syncStatus.textContent='正文读取失败，请刷新来源后重试。';focusAfter=syncRefresh;}throw failure;}finally{if(!editing)releaseInert();}});return;
    }
    const button=event.target.closest('[data-skill-sync-preview]');if(!button||busy||pending||editing||!syncFresh)return;const row=inventory?.entries[Number(button.dataset.skillSyncPreview)];if(!row?.available||row.tool===target.value)return;run(async()=>{constrain();try{const preview=await invoke('skills_package_sync_preview',{id:row.id,generation:inventory.generation,target:target.value});if(request.ownsRequest())show(preview,null,button);}catch(failure){if(request.ownsRequest()){syncFresh=false;syncStatus.textContent='同步预览失败，请刷新来源后重试。';focusAfter=syncRefresh;}throw failure;}finally{if(!pending)releaseInert();}});};
  bodyText.oninput=()=>{bodyStatus.textContent=editing?.attempted?'保存结果需核对；可复制草稿后关闭并刷新。':'草稿未保存';lock();};
  bodyClose.onclick=()=>run(async()=>{if(!editing)return;await invoke('skills_package_edit_close',{ticket:editing.ticket});if(!request.ownsRequest())return;const attempted=editing.attempted;focusAfter=attempted?refresh:editing.origin;editing=null;bodyText.value='';editor.hidden=true;releaseInert();feedback(attempted?'已关闭编辑，请刷新安装记录核对保存结果。':'已关闭编辑，文件未改动。');});
  bodyPreview.onclick=()=>run(async()=>{if(!editing)return;const result=await invoke('skills_package_edit_preview',{ticket:editing.ticket,body:bodyText.value});if(!request.ownsRequest())return;show(result.preview,null,bodyPreview);
    const diff=confirm.querySelector('[data-skill-package-diff]');const block=document.createElement('div');block.className='mcp-field-diff';
    for(const [label,text] of [['当前正文',result.before],['保存后正文',result.after]]){const detail=document.createElement('details'),summary=document.createElement('summary'),pre=document.createElement('pre');summary.textContent=label;pre.textContent=text;detail.append(summary,pre);block.append(detail);}diff.append(block);
  });
  root.addEventListener('skill-package-resume',()=>{if(pending){constrain();lock();}else if(editing){constrain(editor);lock();}});
  root.addEventListener('toggle',event=>{if(event.target!==root||!root.open||started)return;started=true;run(load);});
  choose.onclick=()=>run(async()=>{constrain();try{feedback('请选择根部含 SKILL.md 的目录');const preview=await invoke('skills_package_choose',{target:target.value});if(!request.ownsRequest())return;if(!preview){feedback('已取消目录选择。');return;}show(preview);feedback('请核对文件差异后确认。');}finally{if(!pending)releaseInert();}});
  refresh.onclick=()=>run(load);
  records.onclick=event=>{
    const button=event.target.closest('[data-skill-package-restore],[data-skill-package-trash]');if(!button||busy||!fresh||pending||editing)return;
    const trash=button.hasAttribute('data-skill-package-trash'),row=rows[Number(trash?button.dataset.skillPackageTrash:button.dataset.skillPackageRestore)];if(!row)return;
    run(async()=>{constrain();try{const preview=await invoke(trash?'skills_package_trash_preview':'skills_package_restore_preview',trash?{id:row.id}:{id:row.id,revision:row.revision});if(request.ownsRequest())show(preview,trash?null:row.id,button);}finally{if(!pending)releaseInert();}});
  };
  cancel.onclick=()=>run(async()=>{const old=pending;if(!old)return;if(!old.id)await invoke('skills_package_cancel',{planId:old.preview.plan_id});if(!request.ownsRequest())return;dismiss();if(editing){constrain(editor);focusAfter=bodyText;bodyStatus.textContent=old.attempted?'保存结果需核对；草稿仍保留，可复制后关闭并刷新。':'已取消预览，草稿保留。';}else focusAfter=old.attempted?refresh:caller?.isConnected?caller:refresh;feedback(old.attempted?'已关闭预览，请刷新安装记录核对文件状态。':rows.length>=100?'已取消。记录名额已满，可移出不再需要的记录。':'已取消，文件未改动。');});
  apply.onclick=()=>run(async()=>{
    if(!pending||expired())return;const old=pending;old.attempted=true;if(editing)editing.attempted=true;fresh=false;syncFresh=false;syncStatus.textContent='来源可能已变化，请刷新后再操作。';
    let result;try{result=await invoke(old.preview.action==='trash'?'skills_package_trash':old.id?'skills_package_restore':'skills_package_apply',old.id?{id:old.id,revision:old.preview.plan_id}:{planId:old.preview.plan_id});}
    finally{if(request.ownsRequest())root.dispatchEvent(new CustomEvent('skill-package-changed',{bubbles:true}));}
    if(!request.ownsRequest())return;dismiss();fresh=false;
    if(editing){const ticket=editing.ticket;editing=null;bodyText.value='';editor.hidden=true;try{await invoke('skills_package_edit_close',{ticket});}catch{/* File publication succeeded; abandoned read tickets expire without writes. */}}
    let readNotice='';try{await read();}catch{readNotice=' 安装记录刷新失败，请重试核对；文件操作已完成。';}
    if(syncStarted)await readSync();
    feedback(result.notice+readNotice);
    focusAfter=refresh;
  });
  confirm.onkeydown=event=>{
    if(event.key==='Escape'&&!busy){event.preventDefault();cancel.click();}
    if(event.key==='Tab'){const controls=[...confirm.querySelectorAll('button,summary')].filter(node=>!node.disabled&&node.getClientRects().length);if(!controls.length)return;event.preventDefault();const i=controls.indexOf(document.activeElement);controls[i<0?(event.shiftKey?controls.length-1:0):(i+(event.shiftKey?-1:1)+controls.length)%controls.length].focus();}
  };
}
