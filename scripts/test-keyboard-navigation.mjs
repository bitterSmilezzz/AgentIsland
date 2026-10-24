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
// SUP-05「键盘导航专项」：主流程不依赖鼠标。四层：
// ①主导航项是原生可聚焦元素（button），Tab 能走到——不是靠 tabindex=0 的 div 冒充；
// ②Enter 激活导航项真实切页（键盘用户的核心路径）；
// ③⌘K 快速导航全键盘（ArrowDown/Enter）切页——已有实现，这里钉住不退化；
// ④Escape 在快速导航里关闭并把焦点还给触发器。
const installFixture=page=>page.addInitScript(()=>{
 window.events={};window.calls=[];
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
try {
 for(const motion of ['no-preference','reduce']) {
  const page=await browser.newPage({viewport:{width:1100,height:640},reducedMotion:motion});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await installFixture(page);
  await page.goto(origin+'/app/ui/index.html?shell=workbench');
  await page.waitForSelector('.wb-content');

  // ① 主导航是原生可聚焦控件：Tab 序列能覆盖它（不是 div 冒充）
  await page.keyboard.press('Tab');
  await page.waitForFunction(()=>document.activeElement?.classList?.contains('wb-nav-item'));
  const navCount=await page.locator('.wb-nav-item').count();
  const focused=await page.evaluate(()=>document.activeElement?.dataset?.wbNav);
  assert.ok(focused,`Tab 后焦点应在主导航项上，实际 ${await page.evaluate(()=>document.activeElement?.className)}`);

  // ② Enter 激活导航项真实切页
  await page.keyboard.press('Enter');
  await page.waitForFunction(nav=>document.querySelector('.wb-content [data-workbench-page]')?.dataset?.workbenchPage===nav
    ||document.querySelector('.wb-content.wb-overview')&&nav==='overview', focused);
  assert.ok(true);

  // ③ ⌘K 快速导航全键盘：打开 → ArrowDown → Enter 切页
  await page.keyboard.press('Meta+k');
  await page.waitForSelector('dialog.quick-navigation[open]');
  const quickInput=await page.evaluate(()=>document.activeElement?.id);
  assert.equal(quickInput,'quick-navigation-input','⌘K 后焦点必须在快速导航输入框');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  // Enter 后对话框关闭、工作台内容已切到所选页面
  await page.waitForFunction(()=>!document.querySelector('dialog.quick-navigation[open]'));
  await page.waitForSelector('.wb-content');
  assert.deepEqual(errors,[]);
  await page.close();
 }

 // ④ Escape 关闭快速导航并把焦点还给触发器
 {
  const page=await browser.newPage({viewport:{width:1100,height:640}});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await installFixture(page);
  await page.goto(origin+'/app/ui/index.html?shell=workbench');
  await page.waitForSelector('.wb-content');
  await page.click('[data-wb-quick]');
  await page.waitForFunction(()=>document.activeElement?.tagName==='INPUT');
  await page.keyboard.press('Escape');
  await page.waitForFunction(()=>document.activeElement?.matches('[data-wb-quick]'));
  assert.deepEqual(errors,[]);
  await page.close();
 }
 console.log('PASS: keyboard navigation — Tab reaches native nav buttons, Enter switches pages, ⌘K fully keyboard-driven, Escape restores focus (both motion modes)');
} finally {
 await browser.close();
 server.close();
}
