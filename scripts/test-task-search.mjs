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
// TASK-01「跨项目搜索」：想不起项目归属时按名称或来源找任务。
// 断言四层：跨项目匹配、来源 agent 匹配、与项目过滤/归档开关组合、空结果如实说空。
// 搜索是纯前端过滤：板上数据本就全量，不得因此多一次 tasks_snapshot 请求。
const installFixture=page=>page.addInitScript(()=>{
 window.calls=[];let snapshotCalls=0;
 window.__TAURI__={core:{invoke:async(cmd)=>{
  window.calls.push(cmd);
  if(cmd==='get_boot_args')return {demo:false,expand:false,route:'tasks'};
  if(cmd==='get_settings')return {appearance:'light',shell_mode:'island',dock_edge:'top',sample_interval:2,idle_sample_interval:5,disabled_agents:[],notification_policy:'standard'};
  if(cmd==='get_engine_state')return {snapshots:[],grand_total:{tokens24h:0,tokens:0,cost:0},latest_event:null};
  if(cmd==='window_is_visible')return true;
  if(cmd==='drain_navigation')return [];
  if(cmd==='tasks_attention_summary')return {revision:1,total:0,first:null};
  if(cmd==='tasks_snapshot'){
   snapshotCalls++;
   // 两个项目各两条 + 一条带 codex 来源：搜索必须同时命中两个项目里的同名前缀。
   const tasks=[
    {id:'alpha-1',title:'修复登录页崩溃',source:null,project_id:'p-web',current_run_id:'run-1',archived_ms:null},
    {id:'alpha-2',title:'登录页文案修订',source:null,project_id:'p-web',current_run_id:null,archived_ms:null},
    {id:'beta-1',title:'修复数据库迁移',source:{agent_id:'codex',session_id:'s-1',thread_id:null},project_id:'p-db',current_run_id:'run-2',archived_ms:null},
    {id:'beta-2',title:'归档旧备份脚本',source:null,project_id:'p-db',current_run_id:null,archived_ms:1700000000000},
   ];
   const runs=[{id:'run-1',task_id:'alpha-1',status:'running',started_ms:1700000000000},{id:'run-2',task_id:'beta-1',status:'completed',started_ms:1700000000000}];
   return {revision:snapshotCalls,projects:[{id:'p-web',name:'网页'},{id:'p-db',name:'数据库'}],tasks,runs,attentions:[],artifacts:[]};
  }
  if(cmd==='task_sources')return [];
  return null;
 }},event:{listen:async()=>()=>{}}};
});
try {
 const page=await browser.newPage({viewport:{width:1100,height:640}});
 const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await installFixture(page);
 await page.goto(origin+'/app/ui/index.html?shell=workbench');
 await page.waitForSelector('[data-tasks-root][data-page-ready=true]');
 const titles=()=>page.locator('[data-task-board] .task-row strong').allTextContents();
 // ① 跨项目匹配：两条「修复」分属两个项目，搜索必须同时命中
 assert.equal((await titles()).length,3,'初始视图应显示 3 条未归档任务');
 await page.fill('[data-task-search]','修复');
 await page.waitForFunction(()=>document.querySelectorAll('[data-task-board] .task-row').length===2);
 const matched=await titles();
 assert.ok(matched.includes('修复登录页崩溃')&&matched.includes('修复数据库迁移'),
  `跨项目搜索应同时命中两个项目的任务，实际：${matched}`);
 // ② 来源 agent 匹配：按 codex 找到带来源的任务
 await page.fill('[data-task-search]','codex');
 await page.waitForFunction(()=>document.querySelectorAll('[data-task-board] .task-row').length===1);
 assert.equal((await titles())[0],'修复数据库迁移','按来源 agent 搜索');
 // ③ 与项目过滤组合（AND）：项目限定 p-web 时，跨项目的「修复」只剩网页那条
 await page.fill('[data-task-search]','修复');
 await page.selectOption('[data-task-filter]','p-web');
 await page.waitForFunction(()=>document.querySelectorAll('[data-task-board] .task-row').length===1);
 assert.equal((await titles())[0],'修复登录页崩溃','搜索与项目过滤组合应为 AND');
 // ④ 空结果如实说空，并且不触发新的快照请求（纯前端过滤）
 const snapshotsBefore=(await page.evaluate(()=>window.calls.filter(c=>c==='tasks_snapshot').length));
 await page.fill('[data-task-search]','不存在的词');
 await page.waitForFunction(()=>document.querySelector('[data-task-board] .wb-empty')?.textContent?.includes('没有匹配'));
 assert.equal(await page.evaluate(()=>window.calls.filter(c=>c==='tasks_snapshot').length),snapshotsBefore,
  '搜索过滤不得触发新的 tasks_snapshot——板上数据本来就是全量的');
 // ⑤ 清空恢复全量；与归档开关组合（归档里的「修复」也搜得到）
 await page.selectOption('[data-task-filter]','');
 await page.fill('[data-task-search]','');
 await page.waitForFunction(()=>document.querySelectorAll('[data-task-board] .task-row').length===3);
 await page.check('[data-task-archived]');
 await page.fill('[data-task-search]','归档');
 await page.waitForFunction(()=>document.querySelectorAll('[data-task-board] .task-row').length===1);
 assert.equal((await titles())[0],'归档旧备份脚本','归档视图内搜索');
 // ⑥ 可访问标签在位（键盘用户先于鼠标用户）
 const label=await page.getAttribute('[data-task-search]','aria-label');
 assert.ok(label&&label.includes('搜索'),`搜索框缺 aria-label，实际：${label}`);
 assert.deepEqual(errors,[]);
 await page.close();
 console.log('PASS: cross-project search by title/source composes with project+archive filters; empty is honest; no extra snapshots');
} finally {
 await browser.close();
 server.close();
}
