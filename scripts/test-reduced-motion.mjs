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
// SUP-05「减少动态」：`prefers-reduced-motion: reduce` 必须真的把动效停下来——
// ①JS 驱动的 WAAPI（page-motion / island-navigation 都读媒体查询）在 reduce 下零调用；
// ②CSS 层全局扫描：reduce 下**任何**渲染中的元素都不得有非零 transition 时长——
//   散点的 @media 规则漏了谁，这里就点名谁（防「新增样式忘了配 reduce」）；
// ③对照面：no-preference 下动画**确实在跑**（证明门禁分得出动与不动，不是恒绿）；
// ④功能不受影响：reduce 只是停动效，路由照常切换。
const installFixture=page=>page.addInitScript(()=>{
 window.events={};window.calls=[];
 window.__animateCalls=0;
 const original=Element.prototype.animate;
 Element.prototype.animate=function(...args){window.__animateCalls++;return original.apply(this,args);};
 window.__TAURI__={core:{invoke:async(cmd)=>{
  window.calls.push(cmd);
  if(cmd==='get_boot_args')return {demo:false,expand:false,route:'overview'};
  if(cmd==='get_settings')return {appearance:'light',shell_mode:'island',dock_edge:'top',sample_interval:2,idle_sample_interval:5,disabled_agents:[],notification_policy:'standard'};
  if(cmd==='get_engine_state')return {snapshots:[],grand_total:{tokens24h:0,tokens:0,cost:0},latest_event:null};
  if(cmd==='window_is_visible')return true;
  if(cmd==='drain_navigation')return [];
  if(cmd==='tasks_attention_summary')return {revision:1,total:0,first:null};
  if(cmd==='tasks_snapshot')return {revision:1,projects:[],tasks:[],runs:[],attentions:[],artifacts:[]};
  if(cmd==='task_sources')return [];
  return null;
 }},event:{listen:async(name,fn)=>{(window.events[name]??=[]).push(fn);return ()=>{};}}};
});
const openPage=async motion=>{
 const page=await browser.newPage({viewport:{width:1100,height:640},reducedMotion:motion});
 const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await installFixture(page);
 await page.goto(origin+'/app/ui/index.html?shell=workbench');
 await page.waitForSelector('.wb-content');
 await page.waitForFunction(()=>document.readyState==='complete');
 return {page,errors};
};
try {
 // ── 对照面：no-preference 下页面切换真的有动画 ──
 {
  const {page,errors}=await openPage('no-preference');
  await page.evaluate(()=>{window.__animateCalls=0;});
  await page.click('[data-wb-nav="sessions"]');
  await page.waitForSelector('[data-workbench-page="sessions"]');
  const calls=await page.evaluate(()=>window.__animateCalls);
  assert.ok(calls>0,`no-preference 下页面切换应当产生 WAAPI 动画，实际 ${calls} 次——门禁的对照面失效`);
  assert.deepEqual(errors,[]);
  await page.close();
 }
 // ── reduce：JS 动画零调用 + CSS 全局零 transition + 功能照常 ──
 {
  const {page,errors}=await openPage('reduce');
  const cssOff=await page.evaluate(()=>{
   // ① 已知带 transition 的元素：@media 规则必须把它们关掉
   const navItem=document.querySelector('.wb-nav-item');
   const navOff=navItem&&getComputedStyle(navItem).transitionDuration;
   // ② 全局扫描：渲染中的所有元素，reduce 下 transition 时长必须全为零
   const offenders=[];
   for(const el of document.body.querySelectorAll('*')){
    const style=getComputedStyle(el);
    const durations=(style.transitionDuration||'0s').split(',').map(s=>s.trim());
    if(durations.some(d=>d!=='0s'&&d!=='normal'))offenders.push({tag:el.tagName.toLowerCase(),cls:el.className&&String(el.className).slice(0,60),d:style.transitionDuration});
    if(offenders.length>=5)break;
   }
   return {navOff,offenders};
  });
  assert.equal(cssOff.navOff,'0s',`.wb-nav-item 在 reduce 下 transitionDuration 应为 0s，实际 ${cssOff.navOff}——workbench.css 的 @media 规则漂了`);
  assert.deepEqual(cssOff.offenders,[],`reduce 下这些元素仍在动效（缺 @media (prefers-reduced-motion: reduce) 规则）：${JSON.stringify(cssOff.offenders)}`);
  await page.evaluate(()=>{window.__animateCalls=0;});
  await page.click('[data-wb-nav="sessions"]');
  await page.waitForSelector('[data-workbench-page="sessions"]');
  const calls=await page.evaluate(()=>window.__animateCalls);
  assert.equal(calls,0,`reduce 下页面切换不得调用 WAAPI 动画，实际 ${calls} 次——page-motion / island-navigation 的媒体查询守卫漂了`);
  assert.deepEqual(errors,[]);
  await page.close();
 }
 // ── 岛形态同样过一遍全局扫描（主表面，散点规则最多的地方）──
 {
  const page=await browser.newPage({viewport:{width:360,height:520},reducedMotion:'reduce'});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await installFixture(page);
  await page.goto(origin+'/app/ui/index.html');
  await page.waitForFunction(()=>document.querySelector('.card, #root')&&document.readyState==='complete');
  await page.waitForTimeout(300);
  const offenders=await page.evaluate(()=>{
   const list=[];
   for(const el of document.body.querySelectorAll('*')){
    const style=getComputedStyle(el);
    const durations=(style.transitionDuration||'0s').split(',').map(s=>s.trim());
    if(durations.some(d=>d!=='0s'&&d!=='normal'))list.push({tag:el.tagName.toLowerCase(),cls:el.className&&String(el.className).slice(0,60),d:style.transitionDuration});
    if(list.length>=5)break;
   }
   return list;
  });
  assert.deepEqual(offenders,[],`岛形态 reduce 下这些元素仍在动效：${JSON.stringify(offenders)}`);
  assert.deepEqual(errors,[]);
  await page.close();
 }
 console.log('PASS: reduce suppresses WAAPI and every rendered transition; no-preference still animates; routing unaffected');
} finally {
 await browser.close();
 server.close();
}
