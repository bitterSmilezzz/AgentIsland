import { workspaceFlow } from './workspace-flow.js';
import { invoke } from './tauri.js';
import { pageRequest } from './page-host.js';
import { agentIdentities } from './agent-icons.js';
let requestedRule=null,requestedRecovery=null;
export function selectLayoutRecovery(id){requestedRecovery=id;document.querySelector('[data-layout-root]')?.focusWorkspaceRecovery?.();}
export function selectLayoutRule(id){requestedRule=id;document.querySelector('[data-layout-root]')?.selectWorkspaceRule?.();}
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const labels = { applied: '已排列', failed: '未完成', skipped: '已跳过', restored: '已恢复', conflict: '需确认恢复' };
export function pageWindowLayout() {
  return `<section class="layout-space" data-layout-root>
    <div class="layout-toolbar"><p data-layout-status role="status" aria-live="polite">正在检查窗口能力</p><button type="button" class="mini-btn" data-layout-permission hidden>去授权</button><button type="button" class="mini-btn" data-layout-refresh>读取窗口</button></div>
    <div class="layout-rules"><label class="sr-only" for="layout-rule">已保存布局</label><select id="layout-rule" data-layout-rule><option value="">选择已保存布局</option></select><button type="button" class="mini-btn" data-layout-rule-load disabled>载入</button><button type="button" class="mini-btn" data-layout-rule-remove disabled>删除规则</button><span data-layout-rules-status role="status" aria-live="polite"></span></div>
    <div class="layout-columns"><section class="layout-selection"><h2>选择窗口</h2><p class="layout-note">按点击顺序排列，首个为主窗。</p><div data-layout-windows role="region" aria-label="可排列窗口" tabindex="0"></div></section>
    <section class="layout-arrangement"><div class="layout-controls"><label>目标屏幕<select data-layout-screen aria-label="目标屏幕"></select></label><label>排列方式<select data-layout-template aria-label="排列方式"><option value="side_by_side">左右并排</option><option value="main_and_two">主窗与两辅窗</option><option value="grid">均分网格</option></select></label><label>间距<input type="number" min="0" max="64" step="1" value="12" data-layout-gap aria-label="窗口间距"></label></div>
    <p class="layout-note" data-layout-preview-caption>布局预览</p><div class="layout-canvas" data-layout-canvas aria-label="布局预览"><p>选择窗口后预览</p></div>
    <div class="layout-actions"><button type="button" class="mini-btn" data-layout-preview disabled>预览布局</button><button type="button" class="mini-btn primary" data-layout-apply disabled>应用排列</button></div><p class="layout-note">预览不会移动窗口；应用前会再次检查，预览 15 秒后失效。</p></section></div>
    <section class="layout-results" data-layout-results hidden aria-label="排列结果" tabindex="-1"></section>
    <details class="layout-rule-save"><summary>保存为常用布局</summary><form data-layout-rule-save><label class="sr-only" for="layout-rule-name">布局名称</label><input id="layout-rule-name" name="name" placeholder="布局名称" maxlength="80" required autocomplete="off"><button type="submit" class="mini-btn" data-layout-rule-submit disabled>保存规则</button></form><p class="layout-note">保存排列方式和工具偏好。下次仍需核对窗口、选择屏幕并预览。</p></details>
    <details class="layout-history" data-layout-history><summary>排列历史</summary><p class="layout-note">记录当时的位置与逐窗结果，当前窗口需重新核对。</p><button type="button" class="mini-btn" data-layout-history-refresh>刷新历史</button><p role="status" aria-live="polite" data-layout-history-status>展开后读取本地记录。</p><div data-layout-history-list></div></details>
  </section>`;
}
export function layoutPreviewMarkup(preview, snapshot) {
  const display = snapshot.displays.find(d => d.screen_id === preview.geometry.screen_id);
  if (!display) return '<p>目标屏幕已失效，请重新读取</p>';
  const a = display.rect;
  return `<div class="layout-screen" style="--layout-ratio:${a.width/a.height};aspect-ratio:${a.width}/${a.height}">${preview.geometry.placements.map((p,i) => {
    const w = snapshot.windows.find(w => w.window_id === p.window_id);
    const r = p.target;
    return `<div class="layout-cell${p.restriction ? ' is-restricted' : ''}" style="left:${100*(r.x-a.x)/a.width}%;top:${100*(r.y-a.y)/a.height}%;width:${100*r.width/a.width}%;height:${100*r.height/a.height}%"><span>${i+1}</span><strong>${esc(w?.application ?? '窗口')}</strong>${p.restriction ? `<small>${esc(p.restriction)}</small>` : ''}</div>`;
  }).join('')}</div>`;
}
export function layoutHistoryMarkup(records,snapshot=null){
  if(!records.length)return '<p class="layout-note">还没有排列记录。</p>';
  const rectText=r=>r?`${Math.round(r.x)}, ${Math.round(r.y)} · ${Math.round(r.width)} × ${Math.round(r.height)} pt`:'未核实';
  const recovery=(record,slot,i)=>`<div class="layout-history-recovery"><label>选择当前 ${esc(agentIdentities[slot.agent_id]?.[0]??slot.agent_id)} 窗口<select data-layout-history-window="${esc(slot.id)}" aria-label="历史窗口 ${i+1} 对应的当前窗口"><option value="">选择当前窗口</option>${(snapshot?.windows??[]).filter(w=>w.agent_id===slot.agent_id&&!w.restriction).map(w=>`<option value="${esc(w.window_id)}">${esc(w.application)} · ${esc(w.title||'未命名窗口')}</option>`).join('')}</select></label><button type="button" class="mini-btn" data-layout-history-preview="${esc(record.id)}" data-layout-history-slot="${esc(slot.id)}" disabled>预览恢复</button></div>`;
  const row=record=>`<article class="layout-history-row" data-layout-history-record="${esc(record.id)}"><strong>${esc(new Date(record.created_ms).toLocaleString('zh-CN'))}</strong><span>${esc({pending:'待核对',finished:'已记录结果',failed:'未执行'}[record.phase]??'阶段未核实')}</span><details><summary>位置与结果 · ${record.slots.length} 个窗口</summary>${record.slots.map((slot,i)=>`<div class="layout-history-slot"><strong>${i+1} · ${esc(agentIdentities[slot.agent_id]?.[0]??slot.agent_id)}</strong><small>${esc(labels[slot.outcome]??'结果未核实')}</small><dl><dt>预览原位置</dt><dd>${esc(rectText(slot.before))}</dd><dt>计划位置</dt><dd>${esc(rectText(slot.target))}</dd><dt>执行读回</dt><dd>${esc(rectText(slot.actual))}</dd></dl>${recovery(record,slot,i)}</div>`).join('')}<button type="button" class="mini-btn" data-layout-history-remove="${esc(record.id)}">删除记录</button><button type="button" class="mini-btn" data-layout-history-delete-cancel="${esc(record.id)}" hidden>取消</button><span data-layout-history-delete-note="${esc(record.id)}" role="status"></span></details></article>`;
  const sorted=[...records].sort((a,b)=>b.created_ms-a.created_ms);
  return sorted.slice(0,5).map(row).join('')+(sorted.length>5?`<details><summary>更多记录 · ${sorted.length-5} 条</summary>${sorted.slice(5).map(row).join('')}</details>`:'');
}
export async function hydrateWindowLayout() {
  const root = document.querySelector('[data-layout-root]');
  if (!root || root.layoutReady) return;
  root.layoutReady = true;
  let snapshot = null, selection = [], preview = null, result = null, rules = null, resultTitle = '排列结果', busy = false, expiryTimer = null, loadedRule = null;
  const names = new Map();
  const status = root.querySelector('[data-layout-status]');
  const screen = root.querySelector('[data-layout-screen]');
  const template = root.querySelector('[data-layout-template]');
  const gap = root.querySelector('[data-layout-gap]');
  const ruleSelect=root.querySelector('[data-layout-rule]');
  const ruleStatus=root.querySelector('[data-layout-rules-status]');
  const history=root.querySelector('[data-layout-history]'),historyStatus=root.querySelector('[data-layout-history-status]'),historyList=root.querySelector('[data-layout-history-list]'),historyRefresh=root.querySelector('[data-layout-history-refresh]');
  let historyStarted=false,historyQueued=false,historyData=null,historyFresh=false,recoveryPreview=false,deleteCandidate=null,focusAfter=null,activeRecoveryRecord=null;
  const controls = () => root.querySelectorAll('button,input,select');
  function buttons() {
    for (const e of controls()) e.disabled = busy || e.dataset.layoutUnavailable === 'true';
    root.querySelector('[data-layout-preview]').disabled = busy || !selection.length || !snapshot || !screen.value;
    root.querySelector('[data-layout-apply]').disabled = busy || !preview?.geometry.applicable || Date.now() >= preview.expires_ms;
    root.querySelector('[data-layout-rule-load]').disabled = busy || !rules || !ruleSelect.value || !snapshot;
    root.querySelector('[data-layout-rule-remove]').disabled = busy || !rules || !ruleSelect.value;
    root.querySelector('[data-layout-rule-submit]').disabled = busy || !rules || !snapshot || !selection.length || !screen.value;
    historyList.querySelectorAll('[data-layout-history-preview]').forEach(button=>{const input=[...historyList.querySelectorAll('[data-layout-history-window]')].find(e=>e.dataset.layoutHistoryWindow===button.dataset.layoutHistorySlot);button.disabled=busy||!historyFresh||!snapshot||!input?.value;});
    historyList.querySelectorAll('[data-layout-history-remove],[data-layout-history-window]').forEach(e=>e.disabled=busy||!historyFresh);
  }
  function renderRules() {
    const selected=ruleSelect.value;
    ruleSelect.innerHTML='<option value="">选择已保存布局</option>'+(rules?.items ?? []).map(r=>`<option value="${esc(r.id)}">${esc(r.name)}</option>`).join('');
    if (rules?.items.some(r=>r.id===selected)) ruleSelect.value=selected;
    if(requestedRule && rules){const id=requestedRule;requestedRule=null;if(rules.items.some(r=>r.id===id)){ruleSelect.value=id;ruleStatus.textContent='已选中工作空间布局，载入后预览再应用。';ruleSelect.focus({preventScroll:true});}else ruleStatus.textContent='工作空间的布局已不存在，请刷新核对。';}
  }
  root.selectWorkspaceRule=()=>{if(busy){ruleStatus.textContent='当前操作未结束，请完成后再选择布局。';return;}renderRules();buttons();};
  async function readRules(current) {
    try { const next=await invoke('window_layout_rules_list'); if (current.ownsRequest()) { rules=next; ruleStatus.textContent=''; renderRules(); if(!ruleStatus.textContent)ruleStatus.textContent=next.items.length ? '' : '还没有保存的布局'; } }
    catch(error) { if (current.ownsRequest()) { rules=null; renderRules(); ruleStatus.textContent=String(error); } }
  }
  function clearPreview() { clearTimeout(expiryTimer); expiryTimer=null; preview = null; recoveryPreview=false;activeRecoveryRecord=null;root.querySelector('[data-layout-preview-caption]').textContent='布局预览'; root.querySelector('[data-layout-canvas]').innerHTML = '<p>选择已变化，请重新预览</p>'; buttons(); }
  function windows() {
    const box = root.querySelector('[data-layout-windows]');
    box.innerHTML = snapshot.windows.length ? snapshot.windows.map(w => `<label class="layout-window${w.restriction ? ' is-restricted' : ''}"><input type="checkbox" data-layout-window="${esc(w.window_id)}" ${selection.includes(w.window_id) ? 'checked' : ''} ${w.restriction ? 'disabled data-layout-unavailable="true"' : ''}><span><strong>${esc(w.application)}</strong><small>${esc(w.title)}</small>${w.restriction ? `<em>${esc(w.restriction)}</em>` : ''}</span><b data-layout-order="${esc(w.window_id)}">${selection.includes(w.window_id) ? selection.indexOf(w.window_id)+1 : ''}</b></label>`).join('') : '<p class="layout-empty">没有可选择的工具窗口。打开工具窗口后重新读取。</p>';
    box.querySelectorAll('[data-layout-window]').forEach(input => input.addEventListener('change', () => {
      if (busy) return;
      if (input.checked && selection.length >= 16) { input.checked = false; status.textContent = '一次最多选择 16 个窗口'; return; }
      selection = input.checked ? [...selection, input.dataset.layoutWindow] : selection.filter(id => id !== input.dataset.layoutWindow);
      box.querySelectorAll('[data-layout-order]').forEach(b => { const i=selection.indexOf(b.dataset.layoutOrder); b.textContent=i < 0 ? '' : i+1; });
      loadedRule=null; clearPreview();
    }));
  }
  function results() {
    const box = root.querySelector('[data-layout-results]'); box.hidden = !result;
    if (!result) return;
    box.innerHTML = `<div class="layout-result-head"><h2>${resultTitle}</h2>${result.undo_available ? `<button type="button" class="mini-btn" data-layout-undo>${resultTitle==='历史恢复结果'?'撤销恢复':'撤销排列'}</button>` : ''}</div>${result.record_warning?`<p class="layout-note" role="alert">${esc(result.record_warning)}</p>`:''}${result.windows.map(w => `<div class="layout-result-row"><span><strong>${esc(names.get(w.window_id) ?? '窗口')}</strong><small>${esc(w.reason ?? '')}${w.actual_rect ? ` · ${Math.round(w.actual_rect.width)} × ${Math.round(w.actual_rect.height)}` : ''}</small></span><b>${esc(resultTitle==='历史恢复结果'&&w.status==='applied'?'已恢复':labels[w.status] ?? w.status)}</b>${w.status === 'conflict' ? `<label><input type="checkbox" data-layout-force="${esc(w.window_id)}">覆盖后续调整，恢复原位置</label>` : ''}</div>`).join('')}`;
    box.querySelector('[data-layout-undo]')?.addEventListener('click', () => run(async () => {
      const forceIds = [...box.querySelectorAll('[data-layout-force]:checked')].map(e => e.dataset.layoutForce);
      const next = await workspaceFlow.execute('layout', result.operation_id, args=>invoke('workspace_preview',args), () => invoke('window_layout_undo', { operationId: result.operation_id, forceIds }), true);
      result = next; resultTitle='恢复结果'; results(); focusResult(); status.textContent = next.windows.some(w => w.status === 'conflict') ? '有窗口后来被调整，请逐项确认' : '已检查恢复结果';
    }));
  }
  function focusResult() {
    const box=root.querySelector('[data-layout-results]');
    focusAfter=box.querySelector('[data-layout-force]')??box.querySelector('[data-layout-undo]')??box;
  }
  root.focusWorkspaceRecovery=()=>{
    if(!requestedRecovery)return;
    if(busy)return;const operation=requestedRecovery;requestedRecovery=null;
    const undo=root.querySelector('[data-layout-undo]');
    if(result?.operation_id===operation&&undo){undo.focus({preventScroll:true});undo.scrollIntoView({block:'nearest'});status.textContent='核对逐窗结果后撤销；后续调整需单独确认。';}
    else run(async current=>{
      historyStarted=true;history.open=true;
      await readHistory(current);if(!current.ownsRequest()||!historyFresh)return;
      const record=historyData.items.find(r=>r.id===operation||r.execution_id===operation);
      if(!record){historyStatus.textContent='对应排列记录已不存在或不可核实，请核对工具窗口。';history.open=true;return;}
      history.open=true;const row=[...historyList.querySelectorAll('[data-layout-history-record]')].find(e=>e.dataset.layoutHistoryRecord===record.id);
      for(let parent=row;parent&&parent!==root;parent=parent.parentElement)if(parent.tagName==='DETAILS')parent.open=true;
      row.querySelector('details').open=true;focusAfter=row.querySelector('[data-layout-history-window]');
      row.scrollIntoView({block:'nearest',behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'auto':'smooth'});
      historyStatus.textContent='已定位组合的排列记录。选择当前窗口，预览历史位置后恢复。';
    },historyStatus);
  };
  async function run(action,feedback=status) {
    if (busy) return;
    const origin=root.contains(document.activeElement)?document.activeElement:null;
    busy = true; buttons(); root.setAttribute('aria-busy','true');
    const current = pageRequest(root);
    try { await action(current); } catch (error) { if (current.ownsRequest()) feedback.textContent = error instanceof Error?error.message:String(error); }
    finally {
      if (current.ownsRequest()) {
        busy=false; root.removeAttribute('aria-busy'); buttons();
        const preferred=focusAfter; focusAfter=null;
        if(root.isConnected&&(document.activeElement===document.body||root.contains(document.activeElement))) {
          const target=preferred?.isConnected&&!preferred.disabled?preferred:origin?.isConnected&&!origin.disabled?origin:feedback===historyStatus?historyRefresh:root.querySelector('[data-layout-refresh]');
          if(target?.isConnected&&!target.disabled&&!target.closest('[inert]')) {
            const resultBox=root.querySelector('[data-layout-results]');
            if(preferred===target&&resultBox.contains(target)) {
              (target===resultBox?resultBox.querySelector('.layout-result-head'):target).scrollIntoView({block:'nearest'});
            }
            target.focus({preventScroll:true});
          }
        }
        if(historyQueued){historyQueued=false;run(readHistory,historyStatus);}
        if(requestedRecovery)root.focusWorkspaceRecovery();
      }
    }
  }
  function renderHistory(){if(historyData)historyList.innerHTML=layoutHistoryMarkup(historyData.items,snapshot);buttons();}
  historyList.addEventListener('change',event=>{if(event.target.matches('[data-layout-history-window]')){clearPreview();buttons();}});
  function cancelDelete(){const id=deleteCandidate;deleteCandidate=null;historyList.querySelectorAll('[data-layout-history-remove]').forEach(button=>{button.textContent='删除记录';if(button.dataset.layoutHistoryRemove===id)button.focus({preventScroll:true});});historyList.querySelectorAll('[data-layout-history-delete-cancel]').forEach(button=>button.hidden=true);historyList.querySelectorAll('[data-layout-history-delete-note]').forEach(note=>note.textContent='');}
  root.addEventListener('keydown',event=>{if(event.key==='Escape'&&deleteCandidate&&!busy){event.preventDefault();event.stopPropagation();cancelDelete();}});
  historyList.addEventListener('click',event=>{
    const cancel=event.target.closest('[data-layout-history-delete-cancel]');if(cancel){cancelDelete();return;}
    const remove=event.target.closest('[data-layout-history-remove]');
    if(remove&&!busy&&historyData&&historyFresh){
      const id=remove.dataset.layoutHistoryRemove;
      if(deleteCandidate!==id){deleteCandidate=id;historyList.querySelectorAll('[data-layout-history-delete-cancel]').forEach(button=>button.hidden=button.dataset.layoutHistoryDeleteCancel!==id);historyList.querySelectorAll('[data-layout-history-remove]').forEach(button=>button.textContent=button===remove?'确认删除':'删除记录');historyList.querySelectorAll('[data-layout-history-delete-note]').forEach(note=>note.textContent=note.dataset.layoutHistoryDeleteNote===id?'仅删除历史，不移动窗口；此记录将无法用于恢复。':'');return;}
      run(async current=>{let next;try{next=await invoke('window_layout_history_remove',{id,expectedRevision:historyData.revision});}catch(error){if(current.ownsRequest()){historyFresh=false;cancelDelete();focusAfter=historyRefresh;}throw new Error('删除结果未核实，请刷新历史后核对；不重复删除。');}if(!current.ownsRequest())return;historyData=next;historyFresh=true;focusAfter=historyRefresh;clearPreview();deleteCandidate=null;renderHistory();historyStatus.textContent='已删除历史记录，窗口位置不变。';},historyStatus);return;
    }
    const button=event.target.closest('[data-layout-history-preview]');if(!button||busy||!snapshot||!historyData||!historyFresh)return;
    const input=[...historyList.querySelectorAll('[data-layout-history-window]')].find(e=>e.dataset.layoutHistoryWindow===button.dataset.layoutHistorySlot);if(!input?.value)return;
    run(async current=>{
      clearPreview();loadedRule=null;
      const next=await invoke('window_layout_recovery_preview',{recordId:button.dataset.layoutHistoryPreview,slotId:button.dataset.layoutHistorySlot,windowId:input.value,historyRevision:historyData.revision,expectedRevision:snapshot.revision});
      if(!current.ownsRequest())return;recoveryPreview=true;activeRecoveryRecord=button.dataset.layoutHistoryPreview;showPreview(next);focusAfter=next.geometry.applicable?root.querySelector('[data-layout-apply]'):historyRefresh;status.textContent=next.geometry.applicable?'恢复预览已就绪，核对位置后应用；将覆盖所选窗口的当前位置。':'当前窗口无法恢复到历史位置';
      root.querySelector('[data-layout-canvas]').scrollIntoView({block:'nearest',behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth'});
    },historyStatus);
  });
  function showPreview(next){
    preview=next;root.querySelector('[data-layout-preview-caption]').textContent=recoveryPreview?'历史原位置预览 · 使用记录中的位置与尺寸':'布局预览';root.querySelector('[data-layout-canvas]').innerHTML=layoutPreviewMarkup(next,snapshot);
    expiryTimer=setTimeout(()=>{if(preview===next){preview=null;recoveryPreview=false;status.textContent='预览已过期，请重新预览';buttons();}},Math.max(0,next.expires_ms-Date.now()));
  }
  async function readHistory(current){
    historyStarted=true;historyStatus.textContent='正在读取排列历史';
    try{const next=await invoke('window_layout_history');if(!current.ownsRequest())return;if(!Array.isArray(next?.items))throw new Error('历史格式无法核实');historyData=next;historyFresh=true;deleteCandidate=null;renderHistory();historyStatus.textContent=next.items.length?`已读取 ${next.items.length} 条记录；历史结果不代表当前窗口位置。`:'';}
    catch{if(current.ownsRequest()){historyFresh=false;buttons();historyStatus.textContent=historyData?'历史读取失败，保留上次清单，请刷新重试。':'历史读取失败，请刷新重试。';}}
  }
  history.addEventListener('toggle',event=>{if(event.target!==history||!history.open||historyStarted)return;if(busy){historyQueued=true;historyStatus.textContent='当前操作完成后读取历史';}else run(readHistory,historyStatus);});
  historyRefresh.onclick=()=>run(readHistory,historyStatus);
  async function read(current) {
    loadedRule=null; clearPreview();
    await readRules(current);
    if (!current.ownsRequest()) return;
    status.textContent = '正在读取工具窗口';
    const caps = await invoke('window_layout_capabilities');
    if (!current.ownsRequest()) return;
    root.querySelector('[data-layout-permission]').hidden = !caps.supported || caps.permission_granted;
    if (!caps.permission_granted) { snapshot=null; selection=[]; clearPreview(); root.querySelector('[data-layout-windows]').innerHTML='<p class="layout-empty">允许权限后可读取工具窗口。</p>'; screen.innerHTML=''; status.textContent=caps.reason ?? '当前无法读取窗口'; return; }
    const next = await invoke('window_layout_candidates');
    if (!current.ownsRequest()) return;
    snapshot = next; selection=[]; preview=null;
    next.windows.forEach(w => names.set(w.window_id,w.application));
    screen.innerHTML = '<option value="">选择目标屏幕</option>'+next.displays.map((d,i) => `<option value="${esc(d.screen_id)}">${i === 0 ? '主屏幕' : `屏幕 ${i+1}`} · ${Math.round(d.rect.width)} × ${Math.round(d.rect.height)}</option>`).join('');
    screen.value=next.displays[0]?.screen_id ?? '';
    renderHistory();windows(); root.querySelector('[data-layout-canvas]').innerHTML='<p>选择窗口后预览</p>';
    status.textContent=next.warnings.length ? next.warnings.join('；') : `找到 ${next.windows.length} 个工具窗口`;
  }
  root.querySelector('[data-layout-refresh]').addEventListener('click', () => run(read));
  root.querySelector('[data-layout-permission]').addEventListener('click', () => run(async () => { await invoke('window_layout_open_permissions'); status.textContent='允许 AgentIsland 辅助功能权限后，点击读取窗口'; }));
  [screen,template,gap].forEach(input => input.addEventListener('change',()=>{
    if(input===screen&&loadedRule?.chooseScreen&&screen.value)loadedRule.chooseScreen=false;else loadedRule=null;
    clearPreview();
  }));
  ruleSelect.addEventListener('change',()=>{loadedRule=null;buttons();});
  root.querySelector('[data-layout-rule-save]').addEventListener('submit',event=>{
    event.preventDefault(); if (!snapshot || !rules || !selection.length || !screen.value) return;
    const nameInput=root.querySelector('[name="name"]');
    run(async current=>{
      const next=await invoke('window_layout_save_rule',{name:nameInput.value,selection:[...selection],screenId:screen.value,template:template.value,gap:Number(gap.value),expectedRevision:snapshot.revision,expectedRulesRevision:rules.revision});
      if (!current.ownsRequest()) return;
      rules=next; renderRules(); nameInput.value=''; ruleStatus.textContent='已保存布局';
    },ruleStatus);
  });
  root.querySelector('[data-layout-rule-remove]').addEventListener('click',()=>run(async current=>{
    const next=await invoke('window_layout_remove_rule',{id:ruleSelect.value,expectedRevision:rules.revision});
    if (!current.ownsRequest()) return;
    rules=next; loadedRule=null; renderRules(); ruleStatus.textContent='已删除规则';
  },ruleStatus));
  root.querySelector('[data-layout-rule-load]').addEventListener('click',()=>run(async current=>{
    loadedRule=null; clearPreview();
    const resolved=await invoke('window_layout_resolve_rule',{id:ruleSelect.value,expectedRulesRevision:rules.revision,expectedRevision:snapshot.revision});
    if (!current.ownsRequest()) return;
    template.value=resolved.rule.template; gap.value=resolved.rule.gap;
    screen.value=resolved.rule.screen_preference==='primary' ? snapshot.displays[0]?.screen_id ?? '' : '';
    selection=resolved.selection; loadedRule={id:resolved.rule.id,revision:rules.revision,chooseScreen:!screen.value}; windows();
    ruleStatus.textContent=''; status.textContent=resolved.reason ?? (screen.value ? '布局已载入，核对窗口后预览' : '布局已载入，请选择目标屏幕后预览');
  },ruleStatus));
  root.querySelector('[data-layout-preview]').addEventListener('click', () => run(async current => {
    clearPreview();
    const next=await invoke('window_layout_preview',{selection:[...selection],screenId:screen.value,template:template.value,gap:Number(gap.value),expectedRevision:snapshot.revision});
    if (!current.ownsRequest()) return;
    showPreview(next);
    status.textContent=next.geometry.applicable ? '预览已就绪，应用将调整所选窗口' : '部分窗口不满足布局条件';
  }));
  root.querySelector('[data-layout-apply]').addEventListener('click', () => run(async current => {
    const restoring=recoveryPreview;recoveryPreview=false;const selected=preview; preview=null; clearTimeout(expiryTimer); expiryTimer=null;
    if (!selected || Date.now() >= selected.expires_ms) { status.textContent='预览已过期，请重新预览'; return; }
    status.textContent='正在调整窗口';
    const rule=loadedRule;
    const target=restoring?activeRecoveryRecord:rule?.id;const context=workspaceFlow.layoutContext(target,rules?.revision,restoring);
    const next=await workspaceFlow.execute('layout',target,args=>invoke('workspace_preview',args),async()=>{
      if(rule){const latest=await invoke('window_layout_rules_list');if(latest.revision!==rule.revision)throw new Error('布局规则已变化，请重新载入和预览。');}
      return invoke('window_layout_apply',{previewId:selected.preview_id,expectedRevision:selected.revision,context});
    },restoring);
    if (!current.ownsRequest()) return;
    result=next;if(restoring)root.querySelector('[data-layout-preview-caption]').textContent='历史位置参考 · 执行结果见下方'; resultTitle=restoring?'历史恢复结果':'排列结果'; results();focusResult();status.textContent=restoring?'已执行恢复，请查看逐窗结果':'请查看逐窗结果；再次排列前重新读取窗口';if(historyStarted)await readHistory(current);
  }));
  await run(read);
  root.focusWorkspaceRecovery();
}
