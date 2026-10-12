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
try {
 for (const width of [880,900,1180]) for(const theme of ['light','dark']) for(const motion of ['reduce','no-preference']) {
  const page=await browser.newPage({viewport:{width,height:width===880?560:620}, reducedMotion:motion});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(({theme})=>{
   window.events={};window.calls=[];
   window.__TAURI__={core:{invoke:async(cmd,args)=>{
    calls.push({cmd,args});
    if(cmd==='get_boot_args')return {demo:false,expand:false,route:'tasks'};
    if(cmd==='get_settings')return {appearance:theme,shell_mode:'island',dock_edge:'top',sample_interval:2,idle_sample_interval:5,disabled_agents:[],notification_policy:'standard'};
    if(cmd==='get_engine_state')return {snapshots:[],grand_total:{tokens24h:0,tokens:0,cost:0},latest_event:null};
    if(cmd==='window_is_visible')return true;
    if(cmd==='drain_navigation')return [];
    if(cmd==='tasks_attention_summary')return {revision:1,total:0,first:null};
    if(cmd==='tasks_snapshot')return {revision:1,projects:[],tasks:Array.from({length:16},(_,i)=>({id:'task-'+i,title:'验收任务 '+i,source:null,project_id:null,current_run_id:null,archived_ms:null})),runs:[],attentions:[],artifacts:[]};
    if(cmd==='task_sources')return [{name:'Claude Code',source:{agent_id:'claude-code',session_id:'fixture-session',thread_id:null},observation:{status:'waiting'},target:null}];
    return null;
   }},event:{listen:async(name,fn)=>{(events[name]??=[]).push(fn);return ()=>{};}}};
  },{theme});
  await page.goto(origin+'/app/ui/index.html?shell=workbench');
  await page.waitForSelector('[data-tasks-root][data-page-ready=true]');
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
  await page.locator('[data-task-close]').click();
  await page.waitForTimeout(450);
  assert.equal(await page.locator('[data-task-select="task-0"]').evaluate(n=>n===document.activeElement),true,'closing must return focus to the selected task');
  await page.locator('[data-task-select="task-0"]').click();
  await page.locator('[data-task-edit] input[name=title]').fill('未保存的任务名称');
  await page.locator('[data-task-select="task-1"]').click();
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
  await page.locator('[data-task-archived]').check();
  assert.equal(await page.locator('[data-task-select="task-15"]').count(),0);
  await page.locator('[data-task-close]').click();
  assert.equal(await page.locator('[data-task-filter]').evaluate(n=>n===document.activeElement),true,'a filtered-out origin returns to filters');
  assert.equal(await page.locator('[data-task-archived]').isChecked(),true,'closing must preserve the changed filter');
  assert.equal(errors.length,0,errors.join('\n'));
  await page.close();
 }
 console.log('PASS: task detail navigation, 12 theme/size/motion combinations, routing, drafts and filtered origin');
} finally {
 await browser.close(); await new Promise(r=>server.close(r));
}
