import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve,extname,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
// Reuse an installed browser runtime; this regression installs no dependencies.
const {chromium}=await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const server=createServer(async(req,res)=>{
 try{const path=resolve(root,'.'+decodeURIComponent(new URL(req.url,'http://localhost').pathname));
  if(!path.startsWith(root+'/app/ui/'))throw Error();
  const body=await readFile(path);res.writeHead(200,{'content-type':({'.js':'text/javascript','.css':'text/css','.html':'text/html','.svg':'image/svg+xml'}[extname(path)]??'application/octet-stream')});res.end(body);
 }catch{res.writeHead(404);res.end();}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const origin=`http://127.0.0.1:${server.address().port}`;
const browser=await chromium.launch({headless:true,...(process.env.TEST_CHROMIUM_PATH ? {executablePath:process.env.TEST_CHROMIUM_PATH} : {})});
// Every page gets the same invoke surface, so the container-width boundary below measures
// the same app state as the main matrix instead of a thinner fixture of its own.
const installFixture=(page,theme)=>page.addInitScript(({theme})=>{
 window.events={};window.calls=[];let snapshotCalls=0;
 window.__TAURI__={core:{invoke:async(cmd,args)=>{
  calls.push({cmd,args});
  if(cmd==='get_boot_args')return {demo:false,expand:false,route:'tasks'};
  if(cmd==='get_settings')return {appearance:theme,shell_mode:'island',dock_edge:'top',sample_interval:2,idle_sample_interval:5,disabled_agents:[],notification_policy:'standard'};
  if(cmd==='get_engine_state')return {snapshots:[],grand_total:{tokens24h:0,tokens:0,cost:0},latest_event:null};
  if(cmd==='window_is_visible')return true;
  if(cmd==='drain_navigation')return [];
  if(cmd==='tasks_attention_summary')return {revision:1,total:0,first:null};
  if(cmd==='tasks_snapshot'){
   snapshotCalls++;
   // 45 tasks and 30 open gates on task-0: both panes must really overflow.
   const tasks=Array.from({length:45},(_,i)=>({id:'task-'+i,title:'验收任务 '+i,source:null,project_id:null,current_run_id:'run-'+i,archived_ms:null}));
   const runs=tasks.map((task,i)=>({id:'run-'+i,task_id:task.id,status:'waiting',started_ms:1700000000000+i}));
   const attentions=Array.from({length:30},(_,i)=>({id:'attention-'+i,run_id:'run-0',kind:'answer',state:'open',observed:true,artifact_id:null,manual_resolution:null}));
   // A later snapshot really changes the board, not only its revision.
   for(let i=1;i<snapshotCalls;i++)tasks.push({id:'late-'+i,title:'新到达的任务 '+i,source:null,project_id:null,current_run_id:null,archived_ms:null});
   return {revision:snapshotCalls,projects:[],tasks,runs,attentions,artifacts:[]};
  }
  if(cmd==='task_sources')return [{name:'Claude Code',source:{agent_id:'claude-code',session_id:'fixture-session',thread_id:null},observation:{status:'waiting'},target:null}];
  return null;
 }},event:{listen:async(name,fn)=>{(events[name]??=[]).push(fn);return ()=>{};}}};
},{theme});
try {
 for (const width of [880,900,1180]) for(const theme of ['light','dark']) for(const motion of ['reduce','no-preference']) {
  const page=await browser.newPage({viewport:{width,height:width===880?560:620}, reducedMotion:motion});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await installFixture(page,theme);
  await page.goto(origin+'/app/ui/index.html?shell=workbench');
  await page.waitForSelector('[data-tasks-root][data-page-ready=true]');
  const listWidthBeforeSelect=await page.locator('.task-list-pane').evaluate(n=>n.getBoundingClientRect().width);
  await page.locator('[data-task-select="task-0"]').click();
  await page.waitForFunction(() => {
    const pane=document.querySelector('.wb-content');
    const top=pane.scrollTop, prior=window.scrollProbe;
    window.scrollProbe={top, stable:prior?.top===top ? prior.stable+1 : 0};
    return window.scrollProbe.stable>=8;
  });
  const heading=page.locator('[data-task-detail] > .task-detail-heading');
  const rect=await heading.boundingBox();
  const scroll=await page.locator('.wb-content').evaluate(n=>({top:n.scrollTop,rect:n.getBoundingClientRect().toJSON()}));
  assert(rect.y>=scroll.rect.y && rect.y+rect.height<=scroll.rect.bottom, 'opening a task must reveal its detail heading without manual scrolling');
  const action=await page.locator('[data-task-link] button[type=submit]').boundingBox();
  assert(action.x>=scroll.rect.x && action.x+action.width<=scroll.rect.right && action.y>=scroll.rect.y && action.y+action.height<=scroll.rect.bottom,'source association must be visible');
  assert.equal(await page.locator('[data-task-detail]').evaluate(n=>n===document.activeElement),true,'the opened detail must receive keyboard focus');
  const before=await page.locator('.wb-content').evaluate(n=>n.scrollTop);
  await page.evaluate(()=>document.querySelector('[data-tasks-root]').refreshTasks());
  await page.waitForTimeout(150);
  assert.equal(await page.locator('.wb-content').evaluate(n=>n.scrollTop),before,`background task refresh must not move the viewport (${width}/${theme}/${motion})`);
  // The container query, not isVisible, decides the layout: a wide window that
  // unexpectedly hid the list would otherwise fall silently into the narrow path.
  const containerWidth=await page.locator('.task-space').evaluate(n=>n.getBoundingClientRect().width);
  const twoColumn=containerWidth>=690;
  assert.equal(twoColumn,width===1180,`the task container (${Math.round(containerWidth)}px at ${width}px) must ${width===1180?'keep two columns':'collapse to the detail alone'}`);
  if(twoColumn){
   assert.equal(await page.locator('.task-list-pane').isVisible(),true,'a two-column layout must show the list beside the detail');
   assert.equal(await page.locator('[data-task-detail]').isVisible(),true,'a two-column layout must show the detail beside the list');
   assert.equal(await page.locator('.task-list-pane').evaluate(n=>n.getBoundingClientRect().width),listWidthBeforeSelect,'opening a detail must not resize the two-column list');
   const board=page.locator('.task-board'), detail=page.locator('.task-detail');
   const size=await page.evaluate(()=>{const read=n=>{const e=document.querySelector(n);return {scrollHeight:e.scrollHeight,clientHeight:e.clientHeight};};return {board:read('.task-board'),detail:read('.task-detail')};});
   assert(size.board.scrollHeight>size.board.clientHeight,`the tasks fixture must overflow the list (${size.board.scrollHeight}/${size.board.clientHeight})`);
   assert(size.detail.scrollHeight>size.detail.clientHeight,`the detail fixture must overflow its pane (${size.detail.scrollHeight}/${size.detail.clientHeight})`);
   const settle=selector=>page.waitForFunction(sel=>{const prior=window.paneScrollProbe?.[sel],top=document.querySelector(sel).scrollTop;window.paneScrollProbe={...(window.paneScrollProbe??{}),[sel]:{top,stable:prior&&prior.top===top?prior.stable+1:0}};return window.paneScrollProbe[sel].stable>=5;},selector);
   const position=async()=>({list:await board.evaluate(n=>n.scrollTop),detail:await detail.evaluate(n=>n.scrollTop)});
   const boardBox=await board.boundingBox(), detailBox=await detail.boundingBox();
   await page.mouse.move(boardBox.x+boardBox.width/2,boardBox.y+boardBox.height/2);
   await page.mouse.wheel(0,600); await settle('.task-board');
   const afterListWheel=await position();
   assert(afterListWheel.list>0,`a real wheel over the two-column list must scroll it (${afterListWheel.list})`);
   assert.equal(afterListWheel.detail,0,'scrolling the list must not move the detail');
   await page.mouse.move(detailBox.x+detailBox.width/2,detailBox.y+detailBox.height/2);
   await page.mouse.wheel(0,600); await settle('.task-detail');
   const afterDetailWheel=await position();
   assert(afterDetailWheel.detail>0,`a real wheel over the two-column detail must scroll it (${afterDetailWheel.detail})`);
   assert.equal(afterDetailWheel.list,afterListWheel.list,'scrolling the detail must not move the list');
   assert.equal(await page.locator('.wb-content').evaluate(n=>n.scrollTop),0,'two-column scrolling must stay inside the panes');
   await page.evaluate(()=>document.querySelector('[data-tasks-root]').refreshTasks());
   await settle('.task-board'); await settle('.task-detail');
   const afterRefresh=await position();
   assert((await page.locator('[data-task-select^="late-"]').count())>0,'the refreshed snapshot must really change the board');
   assert.equal(afterRefresh.list,afterListWheel.list,`a background refresh must keep the list scroll position (${width}/${theme}/${motion})`);
   assert.equal(afterRefresh.detail,afterDetailWheel.detail,`a background refresh must keep the detail scroll position (${width}/${theme}/${motion})`);
   const switchTarget=await page.evaluate(()=>{
    const board=document.querySelector('.task-board'), view=board.getBoundingClientRect();
    const row=[...board.querySelectorAll('[data-task-select]')].find(node=>{const r=node.getBoundingClientRect();return r.top>=view.top+4&&r.bottom<=view.bottom-4&&node.dataset.taskSelect!=='task-0';});
    if(!row)return null;const r=row.getBoundingClientRect();
    return {id:row.dataset.taskSelect,x:r.left+r.width/2,y:r.top+r.height/2};
   });
   assert(switchTarget,'a scrolled list must expose another selectable task inside its visible rows');
   await page.mouse.click(switchTarget.x,switchTarget.y);
   await page.waitForFunction(id=>document.querySelector('[data-task-edit] input[name=title]')?.value==='验收任务 '+id.slice(5),switchTarget.id);
   assert.equal(await detail.evaluate(n=>n.scrollTop),0,'selecting another task must start the detail at its top');
   assert.equal(await board.evaluate(n=>n.scrollTop),afterRefresh.list,'selecting another task must keep the list scroll position');
   await page.evaluate(async()=>{const {selectTask}=await import('/app/ui/js/tasks-page.js');selectTask('task-0');});
   await page.waitForFunction(()=>document.querySelector('[data-task-edit] input[name=title]')?.value==='验收任务 0');
  } else {
   assert.equal(await page.locator('.task-list-pane').isVisible(),false,'a single-column layout must hide the list while the detail is open');
   assert.equal(await page.locator('[data-task-detail]').isVisible(),true,'a single-column layout must show the open detail');
  }
  await page.locator('[data-task-close]').click();
  await page.waitForTimeout(450);
  assert.equal(await page.locator('[data-task-select="task-0"]').evaluate(n=>n===document.activeElement),true,'closing must return focus to the selected task');
  if(!twoColumn){
   // The single-column contract is that closing restores the list *and its scroll position*.
   // Only a real wheel produces that state, and a hidden pane reports scrollTop 0, so the
   // position is compared between the open and the closed state, never while hidden.
   const narrowBoard=page.locator('.task-board');
   const narrowBox=await narrowBoard.boundingBox();
   await page.mouse.move(narrowBox.x+narrowBox.width/2,narrowBox.y+narrowBox.height/2);
   await page.mouse.wheel(0,600);
   await page.waitForFunction(()=>{const top=document.querySelector('.task-board').scrollTop,prior=window.narrowScrollProbe;window.narrowScrollProbe={top,stable:prior?.top===top?prior.stable+1:0};return window.narrowScrollProbe.stable>=8;});
   const narrowScrolled=await narrowBoard.evaluate(n=>n.scrollTop);
   assert(narrowScrolled>0,`a real wheel over the single-column list must scroll it (${narrowScrolled})`);
   assert.equal(await page.locator('.task-list-pane').isVisible(),true,'the list must be visible again before a detail is reopened');
   const narrowTarget=await page.evaluate(()=>{
    const board=document.querySelector('.task-board'), view=board.getBoundingClientRect();
    const row=[...board.querySelectorAll('[data-task-select]')].find(node=>{const r=node.getBoundingClientRect();return r.top>=view.top+4&&r.bottom<=view.bottom-4;});
    if(!row)return null;const r=row.getBoundingClientRect();
    return {id:row.dataset.taskSelect,x:r.left+r.width/2,y:r.top+r.height/2};
   });
   assert(narrowTarget,'a scrolled single-column list must expose a selectable task inside its visible rows');
   await page.mouse.click(narrowTarget.x,narrowTarget.y);
   await page.waitForFunction(id=>document.querySelector('[data-task-edit] input[name=title]')?.value==='验收任务 '+id.slice(5),narrowTarget.id);
   assert.equal(await page.locator('.task-list-pane').isVisible(),false,'opening a detail must hide the scrolled single-column list');
   await page.locator('[data-task-close]').click();
   await page.waitForTimeout(450);
   // 这条核对的机制是「浏览器对重新显示的 display:none 子树保留滚动偏移」，不是 app 里
   // 有一段恢复代码（closeDetail 只做 focus/scrollIntoView）。它仍能抓住「app 主动把列表
   // 滚动清零」这类回归；若哪天 Blink 改掉该恢复语义而它失败了，那是浏览器行为变了，
   // 不是产品回归——读到时先核对这一点。
   assert.equal(await narrowBoard.evaluate(n=>n.scrollTop),narrowScrolled,`closing must restore the single-column list scroll position exactly (${width}/${theme}/${motion})`);
   const narrowRestored=await page.locator(`[data-task-select="${narrowTarget.id}"]`).boundingBox();
   const narrowBoardBox=await narrowBoard.boundingBox();
   assert(narrowRestored && narrowRestored.y>=narrowBoardBox.y+4 && narrowRestored.y+narrowRestored.height<=narrowBoardBox.y+narrowBoardBox.height-4,'closing must keep the reopened task inside the restored list viewport');
   assert.equal(await page.locator(`[data-task-select="${narrowTarget.id}"]`).evaluate(n=>n===document.activeElement),true,'closing must return focus to the task opened from the scrolled list');
  }
  await page.locator('[data-task-select="task-0"]').click();
  await page.locator('[data-task-edit] input[name=title]').fill('未保存的任务名称');
  const listVisible=await page.locator('.task-board').isVisible();
  if(listVisible) {
   await page.locator('[data-task-select="task-1"]').click();
  } else {
   // Narrow layouts expose the detail alone; test an incoming route, not a hidden click.
   await page.evaluate(async()=>{const {selectTask}=await import('/app/ui/js/tasks-page.js');selectTask('task-1');});
  }
  assert.equal(await page.locator('[data-task-edit] input[name=title]').inputValue(),'未保存的任务名称','switching must preserve a dirty detail');
  await page.locator('[data-task-close]').click();
  assert.equal(await page.locator('[data-task-detail]').isVisible(),true,'closing must refuse a dirty detail');
  await page.locator('[data-task-edit] [data-task-discard]').click();
  await page.locator('[data-task-close]').click();
  await page.evaluate(async()=>{const {selectTask}=await import('/app/ui/js/tasks-page.js');selectTask('task-15');});
  await page.waitForFunction(()=>document.querySelector('[data-task-edit] input[name=title]')?.value==='验收任务 15');
  await page.waitForFunction(()=>{const top=document.querySelector('.wb-content').scrollTop, prior=window.deepScrollProbe;window.deepScrollProbe={top,stable:prior?.top===top?prior.stable+1:0};return deepScrollProbe.stable>=8;});
  const deep=await heading.boundingBox();
  assert(deep.y>=scroll.rect.y && deep.y+deep.height<=scroll.rect.bottom,'explicit task routing must reveal its heading');
  assert.equal(await page.locator('[data-task-detail]').evaluate(n=>n===document.activeElement),true);
  if(listVisible) {
   await page.locator('[data-task-archived]').check();
  } else {
   // Exercise origin disappearance independently of the narrow layout's visible controls.
   await page.evaluate(()=>{const input=document.querySelector('[data-task-archived]');input.checked=true;input.dispatchEvent(new Event('change',{bubbles:true}));});
  }
  assert.equal(await page.locator('[data-task-select="task-15"]').count(),0);
  await page.locator('[data-task-close]').click();
  assert.equal(await page.locator('[data-task-filter]').evaluate(n=>n===document.activeElement),true,'a filtered-out origin returns to filters');
  assert.equal(await page.locator('[data-task-archived]').isChecked(),true,'closing must preserve the changed filter');
  assert.equal(errors.length,0,errors.join('\n'));
  await page.close();
 }
 // The layout threshold is a *container* query. We pin it by *layout*: find the narrowest
 // window whose panes first become two columns, then check that the container widths either
 // side of that crossing straddle 690. Reading the container width alone would misfire when
 // the window→container mapping jumps (it is not globally 1:1).
 // Known limit: this cannot tell @container from an equivalent @media query — the sampled
 // windows are pixel-equivalent under both. Pinning container semantics would need a case
 // where container width ≠ window width.
 const boundary=await browser.newPage({viewport:{width:920,height:620},reducedMotion:'reduce'});
 await installFixture(boundary,'light');
 await boundary.goto(origin+'/app/ui/index.html?shell=workbench');
 await boundary.waitForSelector('[data-tasks-root][data-page-ready=true]');
 const resizeTo=async width=>{await boundary.setViewportSize({width,height:620});await boundary.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));};
 const containerNow=()=>boundary.locator('.task-space').evaluate(n=>n.getBoundingClientRect().width);
 // 断点的判据是**布局**（轨道数首次变 2），不是容器宽本身：窗口→容器宽的映射不是全局
 // 1:1（侧栏/内边距在个别宽度会跳变），拿容器宽当扫描条件会在映射断裂时误报。
 const columnsNow=()=>boundary.locator('.task-panes').evaluate(n=>getComputedStyle(n).gridTemplateColumns.split(' ').filter(Boolean).length);
 let crossing=null,single=null;
 for(let candidate=920;candidate<1200;candidate++){
  await resizeTo(candidate);
  const measured=await containerNow();
  if(await columnsNow()===2){crossing={window:candidate,container:measured};break;}
  single={window:candidate,container:measured};
 }
 assert(crossing,'the task panes must reach two columns before a 1200px window');
 // 断点必须正好落在 690：单列上界 <690，两列下界 >=690。把 max-width 改成 699 或 679
 // 都会红；而映射跳变（跨度大但仍正确跨越 690）不会误报。
 assert(single.container<690 && crossing.container>=690,`the two-column breakpoint must sit exactly at 690px: widest single-column container ${single.container}px, narrowest two-column container ${crossing.container}px`);
 await boundary.close();
 for(const {width,container,columns} of [{width:single.window,container:single.container,columns:1},{width:crossing.window,container:crossing.container,columns:2}]){
  const page=await browser.newPage({viewport:{width,height:620},reducedMotion:'reduce'});
  await installFixture(page,'light');
  await page.goto(origin+'/app/ui/index.html?shell=workbench');
  await page.waitForSelector('[data-tasks-root][data-page-ready=true]');
  assert.equal(await page.locator('.task-space').evaluate(n=>n.getBoundingClientRect().width),container,`a ${width}px window must give a ${container}px task container`);
  assert.equal(await page.locator('.task-panes').evaluate(n=>getComputedStyle(n).gridTemplateColumns.split(' ').filter(Boolean).length),columns,`a ${container}px container must lay the panes out as ${columns} column(s)`);
  await page.locator('[data-task-select="task-0"]').click();
  await page.waitForTimeout(300);
  assert.equal(await page.locator('.task-list-pane').isVisible(),columns===2,`at a ${container}px container, an open detail must ${columns===2?'keep the list beside it':'replace the list'}`);
  assert.equal(await page.locator('[data-task-detail]').isVisible(),true,`at a ${container}px container the open detail stays visible`);
  await page.close();
 }
 console.log('PASS: task detail navigation, 12 theme/size/motion combinations, two-column independence, routing, drafts and filtered origin');
 console.log(`PASS: container query threshold pinned at ${single.container}px single-column (${single.window}px window) / ${crossing.container}px two-column (${crossing.window}px window), and the narrow list restores its wheel scroll position`);
} finally {
 await browser.close(); await new Promise(r=>server.close(r));
}
