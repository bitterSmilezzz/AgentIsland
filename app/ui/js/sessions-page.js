import { invoke, listen } from './tauri.js';
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const labels = {running:'执行中',waiting:'等待处理',ready:'结果就绪',queued:'未开始',failed:'执行失败',cancelled:'已取消',accepted:'来源已验收'};
const identity = source => JSON.stringify([source.agent_id,source.session_id,source.thread_id ?? null]);

// Observed semantic sources and saved run sources only; never inherit a task's latest source.
export function sessionRows(choices, snapshot, catalog=[]) {
  const rows = new Map();
  const names = new Map(choices.map(choice=>[choice.source.agent_id,choice.name]));
  const add = source => {
    const key = identity(source);
    const row = rows.get(key) ?? {key,source,name:names.get(source.agent_id) ?? source.agent_id,observed:null,catalog:null,records:[]};
    rows.set(key,row); return row;
  };
  for (const choice of choices) {
    const row=add(choice.source); row.name=choice.name; row.observed=choice;
  }
  for(const item of catalog){const row=add(item.source);row.catalog=item;if(!names.has(item.source.agent_id))row.name=item.name;}
  const tasks=new Map(snapshot.tasks.map(task=>[task.id,task]));
  for (const run of snapshot.runs) {
    const task=tasks.get(run.task_id);
    if(!run.source || !task)continue;
    add(run.source).records.push({taskId:task.id,runId:run.id,title:task.title,started:run.started_ms,status:run.status});
  }
  for(const row of rows.values())row.records.sort((a,b)=>b.started-a.started||a.runId.localeCompare(b.runId));
  return [...rows.values()].sort((a,b)=>Number(!!b.observed)-Number(!!a.observed)||Math.max(b.records[0]?.started??0,b.catalog?.modified_ms??0)-Math.max(a.records[0]?.started??0,a.catalog?.modified_ms??0)||a.key.localeCompare(b.key));
}
export function filterSessionRows(rows, {tool='',scope='',query=''}={}) {
  const words=query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return rows.filter(row=>(!tool||row.source.agent_id===tool)&&(!scope||(scope==='observed'?!!row.observed:scope==='catalog'?!!row.catalog:!row.observed&&row.records.length>0))&&words.every(word=>[row.name,row.source.agent_id,row.source.session_id,row.source.thread_id??'',...row.records.map(record=>record.title)].join(' ').toLocaleLowerCase().includes(word)));
}
export function sessionOpenAction(row) {
  if(row.observed)return row.observed.target ? {kind:'observed',label:row.observed.target.exactSession?'打开会话':'打开工具'} : null;
  const record=row.records[0];
  return record ? {kind:'saved',record,label:row.source.agent_id==='codex'&&row.source.thread_id?'打开会话':'打开工具'} : row.catalog?{kind:'catalog',label:row.catalog.target.exactSession?'打开会话':'打开工具'}:null;
}
export const SESSION_BATCH = 50;
export function sessionPage(items, limit = SESSION_BATCH) {
  const count = Number.isFinite(limit) ? Math.max(1, Math.trunc(limit)) : SESSION_BATCH;
  return {items:items.slice(0,count),shown:Math.min(items.length,count),total:items.length,more:items.length>count};
}
export function sessionRecordsHtml(row, limit=SESSION_BATCH, blocked=false) {
  const page=sessionPage(row.records,limit);
  return `<ol>${page.items.map(record=>`<li><div><strong title="${esc(record.title)}">${esc(record.title)}</strong><small>记录状态：${esc(labels[record.status]??'未知')} · ${esc(new Date(record.started).toLocaleString())}</small></div><button type="button" class="mini-btn" data-session-record-task="${esc(record.runId)}" data-session-record-source="${esc(row.key)}" aria-label="查看任务：${esc(record.title)}"${blocked?' disabled':''}>查看任务</button></li>`).join('')}</ol>${page.more?`<button type="button" class="mini-btn" data-session-record-more="${esc(row.key)}">更多记录 · ${page.shown}/${page.total}</button>`:''}`;
}
export function sessionListHtml(rows, blocked=false, history=new Map(), catalogBlocked=false) {
  return rows.length ? `<ul class="session-list">${rows.map(row=>{
    const action=sessionOpenAction(row), latest=row.records[0],saved=history.get(row.key);
    const status=row.observed ? labels[row.observed.observation.status]??'状态未知' : row.records.length?'历史记录':row.catalog?.archived?'归档会话':'目录记录';
    return `<li data-session-row="${esc(row.key)}"><div class="session-row-heading"><strong>${esc(row.name)}</strong><span class="session-badge">${esc(status)}</span></div><p title="${esc(latest?.title ?? row.source.thread_id ?? '')}">${row.observed?'当前观测':row.records.length?'已保存来源':'本机目录'}${latest?` · ${esc(latest.title)}`:` · 会话 ${esc((row.source.thread_id??row.source.session_id).slice(-12))}`}</p><div class="session-row-footer"><span>${!row.observed&&latest?esc(new Date(latest.started).toLocaleString()):!row.observed&&row.catalog?.modified_ms?`目录更新时间 · ${esc(new Date(row.catalog.modified_ms).toLocaleString())}`:''}</span><div>${latest?`<button type="button" class="mini-btn" data-session-task="${esc(row.key)}"${blocked?' disabled':''}>查看任务</button>`:''}${action?`<button type="button" class="mini-btn" data-session-open="${esc(row.key)}" aria-label="${esc(action.label)}：${esc(row.name)}${latest?` · ${esc(latest.title)}`:` · ${esc(row.source.thread_id??row.source.session_id)}`}"${blocked||(action.kind==='catalog'&&catalogBlocked)?' disabled':''}>${esc(action.label)}</button>`:'<span>无桌面入口</span>'}</div></div>${row.records.length?`<details class="session-records"${saved?.open?' open':''}><summary data-session-history="${esc(row.key)}">${row.records.length} 条运行记录</summary><div data-session-record-list>${saved?.open?sessionRecordsHtml(row,saved.limit,blocked):''}</div></details>`:''}</li>`;
  }).join('')}</ul>` : '<div class="wb-empty">没有匹配的会话来源</div>';
}
export function catalogSummary(catalog){
  const gaps=catalog.gaps;
  const details=[gaps.unreadable?`${gaps.unreadable} 项无法读取`:null,gaps.invalid?`${gaps.invalid} 项无法核验`:null,gaps.compressed?`${gaps.compressed} 项压缩记录暂不支持`:null,gaps.deep_directories?`${gaps.deep_directories} 个深层目录未扫描`:null,gaps.missing_roots?`${gaps.missing_roots} 个目录不存在`:null].filter(Boolean);
  return `Codex 默认目录 · ${catalog.items.length} 个可核验记录${details.length?'；'+details.join('，'):''}。目录记录不表示正在运行。`;
}
export function pageSessions() {
  return `<section class="session-space" data-sessions-root><div class="session-controls"><input type="search" aria-label="搜索会话来源" data-session-query placeholder="搜索工具、会话编号或任务"><select aria-label="筛选会话工具" data-session-tool><option value="">全部工具</option></select><select aria-label="筛选会话来源" data-session-scope><option value="">全部来源</option><option value="observed">当前观测</option><option value="saved">任务历史</option><option value="catalog">本机目录</option></select><button type="button" class="mini-btn" data-session-refresh>刷新</button><button type="button" class="mini-btn" data-session-catalog-read>读取本机目录</button></div><p class="session-feedback" data-session-feedback role="status" aria-live="polite">正在读取来源</p><p data-session-catalog-feedback class="session-feedback" role="status" aria-live="polite" hidden></p><div data-session-list></div><button type="button" class="mini-btn session-more" data-session-more hidden>显示更多</button><p class="session-note">当前语义来源、任务历史及手动读取的本机目录分开标注。目录目前仅支持 Codex 默认目录中的未压缩记录；“打开工具”无法定位具体会话。</p></section>`;
}
let subscribed;
export function refreshSessions(){document.querySelector('[data-sessions-root]')?.refreshSessions?.();}
export async function hydrateSessions(openTask, isVisible=()=>true) {
  const root=document.querySelector('[data-sessions-root]');if(!root)return;
  if(root.refreshSessions){root.refreshSessions();return;}
  let rows=[], snapshot=null, busy=false, loading=false, queued=false, readError=false, limit=SESSION_BATCH, lastMarkup=null;
  const history=new Map();
  let choices=[],catalog=null,catalogReading=false,catalogError=false;
  const focusAttributes=['data-session-open','data-session-task','data-session-record-task','data-session-record-more','data-session-history','data-session-more','data-session-catalog-read'];
  const captureFocus=()=>{const node=document.activeElement;if(!root.contains(node))return null;const attr=focusAttributes.find(attr=>node.hasAttribute(attr));return attr?{attr,value:node.getAttribute(attr)}:null;};
  const restoreFocus=token=>{if(token)[...root.querySelectorAll(`[${token.attr}]`)].find(node=>node.getAttribute(token.attr)===token.value&&!node.disabled)?.focus({preventScroll:true});};
  const list=root.querySelector('[data-session-list]'),feedback=root.querySelector('[data-session-feedback]');
  const active=()=>root.isConnected&&isVisible();
  const render=()=>{
    const token=captureFocus();
    const filtered=filterSessionRows(rows,{tool:root.querySelector('[data-session-tool]').value,scope:root.querySelector('[data-session-scope]').value,query:root.querySelector('[data-session-query]').value});
    const page=sessionPage(filtered,limit),markup=sessionListHtml(page.items,busy||loading||readError,history,catalogReading||catalogError);
    if(markup!==lastMarkup){list.innerHTML=markup;lastMarkup=markup;restoreFocus(token);}
    root.querySelector('[data-session-catalog-read]').disabled=busy||loading||catalogReading;
    const more=root.querySelector('[data-session-more]');more.hidden=!page.more;more.textContent=`显示更多 · ${page.shown}/${page.total}`;
    if(!busy&&!loading&&!readError){const text=page.more?`显示 ${page.shown} / ${page.total} 个来源`:`${page.total} 个来源`;if(feedback.textContent!==text)feedback.textContent=text;}
  };
  const refresh=async(manual=false)=>{
    if(!active())return;
    if(loading||busy){queued=manual||queued==='manual'?'manual':'automatic';return;}
    const token=captureFocus();
    loading=true;root.querySelector('[data-session-refresh]').disabled=true;
    for(const button of list.querySelectorAll('[data-session-open],[data-session-task],[data-session-record-task]'))button.disabled=true;
    if(manual||!snapshot)feedback.textContent='正在刷新来源';
    try {
      const [observed,data]=await Promise.all([invoke('task_sources'),invoke('tasks_snapshot')]);
      if(!active())return;
      choices=observed;rows=sessionRows(choices,data,catalog?.items??[]);snapshot=data;readError=false;
      const select=root.querySelector('[data-session-tool]'),value=select.value;
      const tools=[...new Map(rows.map(row=>[row.source.agent_id,row.name])).entries()].sort((a,b)=>a[1].localeCompare(b[1]));
      select.innerHTML='<option value="">全部工具</option>'+tools.map(([id,name])=>`<option value="${esc(id)}">${esc(name)}</option>`).join('');
      // A disappeared tool remains a filter, rather than silently broadening the results.
      if(value&&!tools.some(([id])=>id===value))select.insertAdjacentHTML('beforeend',`<option value="${esc(value)}">${esc(value)} · 暂无来源</option>`);
      select.value=value;
    } catch {if(active()){readError=true;feedback.textContent=snapshot?'读取失败，保留上次内容。请刷新后再打开来源。':'来源读取失败，请重试。';}}
    finally {loading=false;for(const button of list.querySelectorAll('[data-session-open],[data-session-task],[data-session-record-task]')){const row=rows.find(row=>row.key===button.dataset.sessionOpen);button.disabled=busy||readError||(row&&sessionOpenAction(row)?.kind==='catalog'&&(catalogError||catalogReading));}root.querySelector('[data-session-refresh]').disabled=false;if(active()){render();if(document.activeElement===document.body)restoreFocus(token);}if(queued){const manual=queued==='manual';queued=false;refresh(manual);}}
  };
  const readCatalog=async()=>{
    if(!active()||catalogReading||busy||loading)return;
    const token=captureFocus();
    catalogReading=true;render();
    const note=root.querySelector('[data-session-catalog-feedback]');note.hidden=false;note.textContent='正在读取 Codex 默认会话目录';
    try{
      const result=await invoke('session_catalog_read');
      // Cached pages may be detached while reading; settle their result before returning.
      catalog=result;catalogError=false;
      rows=sessionRows(choices,snapshot??{tasks:[],runs:[]},catalog.items);
      note.textContent=catalogSummary(catalog);
      // Reuse the normal refresh to update tool options and observed/task references.
      await refresh();
    }catch{catalogError=true;note.textContent=catalog?'目录读取失败，保留上次内容。请重新读取。':'目录读取失败，请重试。';}
    finally{catalogReading=false;if(active()){render();if(document.activeElement===document.body)restoreFocus(token);}}
  };
  root.querySelector('[data-session-catalog-read]').onclick=readCatalog;
  root.refreshSessions=refresh;root.dataset.pageReady='true';
  root.querySelector('[data-session-refresh]').onclick=()=>refresh(true);
  root.querySelectorAll('input,select').forEach(control=>control.addEventListener(control.tagName==='INPUT'?'input':'change',()=>{limit=SESSION_BATCH;render();}));
  root.querySelector('[data-session-more]').onclick=()=>{const previous=limit;limit+=SESSION_BATCH;render();if(root.querySelector('[data-session-more]').hidden)list.querySelectorAll('[data-session-row]')[previous]?.querySelector('button,summary')?.focus({preventScroll:true});};
  list.addEventListener('toggle',event=>{
    const detail=event.target;if(!detail.matches?.('.session-records'))return;
    const key=detail.querySelector('[data-session-history]').dataset.sessionHistory,row=rows.find(row=>row.key===key);if(!row)return;
    const saved=history.get(key)??{open:false,limit:SESSION_BATCH};if(saved.open===detail.open)return;
    history.set(key,{...saved,open:detail.open});lastMarkup=null;
    detail.querySelector('[data-session-record-list]').innerHTML=detail.open?sessionRecordsHtml(row,saved.limit,busy||loading||readError):'';
  },true);
  list.onclick=async event=>{
    const button=event.target.closest('button');if(!button||button.disabled)return;
    if(button.dataset.sessionRecordMore!=null){const key=button.dataset.sessionRecordMore,saved=history.get(key);if(saved){history.set(key,{...saved,limit:saved.limit+SESSION_BATCH});render();if(document.activeElement===document.body){const record=rows.find(row=>row.key===key)?.records[saved.limit];if(record)[...list.querySelectorAll('[data-session-record-task]')].find(node=>node.dataset.sessionRecordTask===record.runId&&node.dataset.sessionRecordSource===key)?.focus({preventScroll:true});}}return;}
    if(busy||loading||readError)return;
    const key=button.dataset.sessionOpen??button.dataset.sessionTask??button.dataset.sessionRecordSource,row=rows.find(row=>row.key===key);if(!row)return;
    if(button.dataset.sessionRecordTask!=null){const record=row.records.find(record=>record.runId===button.dataset.sessionRecordTask);if(record)openTask(record.taskId);return;}
    if(button.dataset.sessionTask!=null){if(row.records[0])openTask(row.records[0].taskId);return;}
    const action=sessionOpenAction(row);if(!action)return;
    const token=captureFocus();
    busy=true;render();feedback.textContent='正在打开来源';
    try {
      const target=await invoke(action.kind==='observed'?'session_open_observed':action.kind==='catalog'?'session_catalog_open':'task_open_source',action.kind==='observed'?{source:row.source}:action.kind==='catalog'?{source:row.source,generation:catalog.generation}:{id:action.record.taskId,runId:action.record.runId,expectedRevision:snapshot.revision});
      if(active())feedback.textContent=target.exactSession?'已打开对应会话':'已打开工具，请在工具内选择会话。';
    }catch{if(active()){if(action.kind==='catalog'){catalogError=true;feedback.textContent='目录来源无法打开，请重新读取本机目录后重试。';}else feedback.textContent='来源无法打开，可能已变化或工具不可用。请刷新后重试。';}}
    finally{busy=false;lastMarkup=null;root.querySelector('[data-session-catalog-read]').disabled=catalogReading;for(const control of list.querySelectorAll('[data-session-open],[data-session-task],[data-session-record-task]')){const row=rows.find(row=>row.key===control.dataset.sessionOpen);control.disabled=readError||(row&&sessionOpenAction(row)?.kind==='catalog'&&(catalogError||catalogReading));}if(active()&&document.activeElement===document.body)restoreFocus(token);if(queued){const manual=queued==='manual';queued=false;refresh(manual);}}
  };
  if(!subscribed)subscribed=Promise.all(['tasks://changed','tasks://sources_changed'].map(name=>listen(name,refreshSessions))).catch(()=>{subscribed=null;});
  await refresh();
}
