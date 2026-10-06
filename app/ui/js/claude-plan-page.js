import {invoke} from './tauri.js';
const esc=value=>String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const labels={running:'采集中',paused:'已暂停',changed:'待核对',uncertain:'未核实',unavailable:'不可用'};
const actions={enable:'配置采集',disable:'移除配置',restore:'还原配置',trash_backup:'移入废纸篓'};
export function pageClaudePlan(){return `<details class="claude-plan" data-claude-plan><summary><span>方案采集</span><small data-plan-state>按需启用</small><svg class="claude-plan-chevron" aria-hidden="true" width="12" height="12" viewBox="0 0 12 12"><path d="m3 4.5 3 3 3-3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg></summary><div class="claude-plan-body"><p class="model-scope">Claude Code · 只读方案 · 正文暂存 15 分钟</p><div data-plan-controls><p class="model-scope">展开后读取状态。</p></div><section class="claude-plan-confirm" data-claude-confirm hidden tabindex="-1" aria-label="配置预览"><strong data-plan-title></strong><p data-plan-scope></p><p data-plan-notice></p><div class="actions"><button type="button" class="mini-btn primary" data-plan-action="confirm">确认</button><button type="button" class="mini-btn" data-plan-action="cancel">取消</button></div></section><p class="model-scope" role="status" aria-live="polite" data-plan-feedback hidden></p></div></details>`;}
export function planControls(status,blocked=false){
  const config=status?.configuration;
  if(!config)return `<p class="model-scope">${esc(status?.notice??'状态尚未读取。')}</p><button type="button" class="mini-btn" data-plan-action="refresh">重新读取</button>`;
  const button=(action,label,disabled=false)=>`<button type="button" class="mini-btn" data-plan-action="${action}"${disabled?' disabled':''}>${label}</button>`;
  const controls=button('refresh','刷新')+(status.state==='running'?button('pause','暂停',blocked):button('resume','启动采集',blocked||!status.resume_ready))+
    (!status.configuration_current&&config.writable?button('enable',config.installed?'更新配置':'配置预览',blocked):'')+(config.installed&&config.writable?button('disable','移除预览',blocked):'');
  const backups=Array.isArray(config.backups)?config.backups:[];
  const records=Array.isArray(config.backup_records)?config.backup_records:backups.map(id=>({id}));
  const row=(record)=>{const index=backups.indexOf(record.id);if(index<0)return '';const date=record.created_ms>0?new Intl.DateTimeFormat('zh-CN',{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'}).format(new Date(record.created_ms)):'快照';return `<div class="connection-row"><div><span>${esc(date)}</span><small>${record.configured_before===undefined?'':record.configured_before?'已配置':'未配置'} · ${esc(record.id.replace('backup-','').slice(0,8))}</small></div><div class="actions">${button(`restore:${esc(record.id)}`,record.restore_available===false?'版本不符':'还原预览',blocked||!config.writable||record.restore_available===false)}${button(`trash_backup:${esc(record.id)}`,'清理预览',blocked)}</div></div>`;};
  const rows=records.slice(0,4).map(row).join('')+(records.length>4?`<details><summary>其余记录 · ${records.length-4}</summary>${records.slice(4).map(row).join('')}</details>`:'');
  return `<p class="model-scope">${esc(status.notice)}</p><div class="actions">${controls}</div>${status.state==='running'?`<small class="model-scope">本次接收 ${Number(status.received)||0} · 拒绝 ${Number(status.rejected)||0}</small>`:''}${backups.length?`<details class="claude-plan-records"><summary>恢复记录 · ${backups.length} / 20</summary>${rows}</details>`:''}`;

}
const bound=new WeakMap();
export function hydrateClaudePlan(root){
  if(!root)return;
  if(bound.has(root))return;
  let status=null,preview=null,busy=false,uncertain=false,origin=null,request=0,reading=null,lastControls=null;
  const controls=root.querySelector('[data-plan-controls]'),confirm=root.querySelector('[data-claude-confirm]'),feedback=root.querySelector('[data-plan-feedback]'),badge=root.querySelector('[data-plan-state]');
  const report=text=>{feedback.textContent=text??'';feedback.hidden=!text;};
  const focusAction=action=>Array.from(root.querySelectorAll('[data-plan-action]')).find(button=>button.dataset.planAction===action)?.focus({preventScroll:true});
  const draw=()=>{
    const focused=controls.contains(document.activeElement)?document.activeElement.dataset.planAction:null;
    const folders=Array.from(controls.querySelectorAll('details')).map(folder=>folder.open);
    root.setAttribute('aria-busy',String(busy));badge.textContent=busy?'处理中…':(labels[status?.state]??'待读取');
    const markup=planControls(status,busy||uncertain||!!preview);
    if(markup!==lastControls){
      controls.innerHTML=markup;lastControls=markup;
      controls.querySelectorAll('details').forEach((folder,index)=>folder.open=folders[index]??false);
      if(focused)focusAction(focused);
    }
    controls.querySelector('[data-plan-action="refresh"]')?.toggleAttribute('disabled',busy);
    confirm.hidden=!preview;confirm.querySelectorAll('button').forEach(button=>button.disabled=busy);
  };
  const visible=()=>root.isConnected&&root.open&&!root.closest('[hidden]');
  const restoreFocus=action=>{if(visible())focusAction(action);else if(root.isConnected&&!root.closest('[hidden]'))root.querySelector('summary').focus();};
  async function readStatus(clearFeedback=false){
    // Await the previous observation before issuing the final read after a mutation.
    if(reading){try{await reading;}catch{/* The fresh read below owns its own error feedback. */}}
    const token=++request;
    const pending=invoke('claude_plan_status');reading=pending;
    try{
      const next=await pending;if(token!==request)return;
      status=next;uncertain=next.state==='uncertain';if(clearFeedback&&!uncertain)report('');
    }catch(error){if(token===request){status=null;report(error?.notice??'状态未读取，请重试。');}}
    finally{if(reading===pending)reading=null;}
  }
  async function refresh(silent=false){
    if(busy||reading||(silent&&preview))return;
    if(silent){await readStatus();if(!busy)draw();return;}
    busy=true;draw();try{await readStatus(true);}finally{busy=false;draw();}
  }
  async function cancel(){
    if(busy)return;busy=true;++request;const focus=origin;preview=null;draw();
    try{await invoke('claude_plan_cancel');report('已取消，配置未修改。');}
    catch{uncertain=true;report('取消未确认，请重新读取。');}
    finally{busy=false;draw();restoreFocus(focus);}
  }
  async function act(action,button){
    if(busy)return;
    if(action==='refresh'){if(preview)await cancel();await refresh();restoreFocus('refresh');return;}
    if(action==='cancel'){await cancel();return;}
    if(uncertain)return;
    busy=true;++request;origin=button.dataset.planAction;draw();
    try{
      if(action==='confirm'){
        const id=preview?.plan_id;if(!id)return;
        if(preview.action!=='trash_backup')window.dispatchEvent(new Event('claude-plan:changed'));
        preview=null;const applied=await invoke('claude_plan_apply',{planId:id});report(applied.notice??'结果已核实。');
      }else if(action==='resume'){
        status=await invoke('claude_plan_resume',{revision:status.configuration.revision});report('采集已启动。');
      }else if(action==='pause'){
        window.dispatchEvent(new Event('claude-plan:changed'));status=await invoke('claude_plan_pause');report('采集已暂停，临时正文已清空。');
      }else{
        const [kind,id]=action.split(':');
        if(id!==undefined&&!status.configuration.backups.includes(id))throw {applied:false,notice:'记录已变化，请重新读取。'};
        preview=await invoke('claude_plan_preview',{action:kind,revision:status.configuration.revision,backupId:id??null});
        confirm.querySelector('[data-plan-title]').textContent=actions[kind]??'配置预览';
        confirm.querySelector('[data-plan-scope]').textContent=preview.scope;
        confirm.querySelector('[data-plan-notice]').textContent=preview.notice;report('');
        if(!visible()){
          preview=null;await invoke('claude_plan_cancel');report('已取消，配置未修改。');
        }
      }
    }catch(error){preview=null;uncertain=error?.applied!==false;report(error?.notice??'执行结果未核实，请重新读取。');}
    finally{
      if(!preview){const message=feedback.textContent;await readStatus();if(message)report(message);}
      busy=false;draw();
      if(preview&&visible())confirm.focus();else restoreFocus('refresh');
    }
  }
  root.addEventListener('click',event=>{const button=event.target.closest('[data-plan-action]');if(button&&!button.disabled)act(button.dataset.planAction,button);});
  root.addEventListener('keydown',event=>{if(event.key==='Escape'&&preview&&!busy){event.preventDefault();event.stopPropagation();cancel();}});
  root.addEventListener('toggle',()=>{if(root.open){refresh();}else if(preview&&!busy){cancel().then(()=>root.querySelector('summary').focus());}});
  const poll=()=>{if(!root.isConnected)return;if(root.open&&!document.hidden&&!root.closest('[hidden]'))refresh(true);setTimeout(poll,5000);};
  bound.set(root,{refresh});if(root.open)refresh();setTimeout(poll,5000);
}
