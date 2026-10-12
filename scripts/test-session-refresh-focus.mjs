import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve,extname,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';

const {chromium}=await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const server=createServer(async(req,res)=>{
 try {
  const path=resolve(root,'.'+decodeURIComponent(new URL(req.url,'http://localhost').pathname));
  if(!path.startsWith(root+'/app/ui/'))throw Error();
  const body=await readFile(path);
  res.writeHead(200,{'content-type':({'.js':'text/javascript','.css':'text/css','.html':'text/html','.svg':'image/svg+xml'}[extname(path)]??'application/octet-stream')});res.end(body);
 }catch{res.writeHead(404);res.end();}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await chromium.launch({headless:true,...(process.env.TEST_CHROMIUM_PATH?{executablePath:process.env.TEST_CHROMIUM_PATH}:{})});
try {
 for(const outcome of ['success','failure'])for(const moveFocus of [false,true]) {
  const page=await browser.newPage({viewport:{width:900,height:620}});
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.addInitScript(()=>{
   window.events={};window.pendingRefresh=null;
   window.__TAURI__={core:{invoke:async(cmd)=>{
    if(cmd==='get_boot_args')return {demo:false,expand:false,route:'sessions'};
    if(cmd==='get_settings')return {appearance:'light',shell_mode:'island',dock_edge:'top',sample_interval:2,idle_sample_interval:5,disabled_agents:[],notification_policy:'standard'};
    if(cmd==='get_engine_state')return {snapshots:[],grand_total:{tokens24h:0,tokens:0,cost:0},latest_event:null};
    if(cmd==='window_is_visible')return true;
    if(cmd==='drain_navigation')return [];
    if(cmd==='tasks_attention_summary')return {revision:1,total:0,first:null};
    if(cmd==='task_sources'){
     if(window.holdRefresh)return new Promise((resolve,reject)=>{window.pendingRefresh={resolve,reject};});
     return [];
    }
    if(cmd==='tasks_snapshot')return {revision:1,projects:[],tasks:[],runs:[],attentions:[],artifacts:[]};
    return null;
   }},event:{listen:async(name,fn)=>{(events[name]??=[]).push(fn);return ()=>{};}}};
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/app/ui/index.html?shell=workbench`);
  await page.waitForSelector('[data-sessions-root][data-page-ready=true]');
  const refresh=page.locator('[data-session-refresh]');
  await page.waitForFunction(()=>!document.querySelector('[data-session-refresh]').disabled);
  await page.evaluate(()=>{window.holdRefresh=true;const button=document.querySelector('[data-session-refresh]');button.focus({preventScroll:true});button.click();});
  await page.waitForFunction(()=>window.pendingRefresh!==null);
  assert.equal(await refresh.isDisabled(),true,'a pending refresh must disable its initiator');
  // WebKit drops focus when a button is disabled. Explicit blur also exercises
  // that platform behavior when this browser keeps focus on the disabled node.
  await page.evaluate(({moveFocus})=>{
   document.querySelector('[data-session-refresh]').blur();
   if(moveFocus)document.querySelector('[data-session-query]').focus({preventScroll:true});
  },{moveFocus});
  await page.evaluate(outcome=>{
   window.holdRefresh=false;
   if(outcome==='failure')window.pendingRefresh.reject(new Error('fixture unavailable'));
   else window.pendingRefresh.resolve([]);
  },outcome);
  await page.waitForFunction(()=>!document.querySelector('[data-session-refresh]').disabled);
  const expected=moveFocus?'[data-session-query]':'[data-session-refresh]';
  assert.equal(await page.locator(expected).evaluate(node=>node===document.activeElement),true,`${outcome}: refresh must restore its initiator without stealing an explicitly moved focus`);
  assert.equal(errors.length,0,errors.join('\n'));
  await page.close();
 }
 console.log('PASS: session refresh restores lost focus after success/failure and preserves user focus changes');
}finally{await browser.close();await new Promise(r=>server.close(r));}
