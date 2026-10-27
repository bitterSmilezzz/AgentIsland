import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve,extname,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
const {chromium}=await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const base=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const server=createServer(async(req,res)=>{try {const path=resolve(base,'.'+decodeURIComponent(new URL(req.url,'http://localhost').pathname));if(!path.startsWith(base+'/app/ui/'))throw Error();const bytes=await readFile(path);res.writeHead(200,{'content-type':({'.js':'text/javascript','.css':'text/css','.html':'text/html','.svg':'image/svg+xml'}[extname(path)]??'application/octet-stream')});res.end(bytes);}catch{res.writeHead(404);res.end();}});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const browser=await chromium.launch({headless:true,...(process.env.TEST_CHROMIUM_PATH ? {executablePath:process.env.TEST_CHROMIUM_PATH} : {})});
try {
 const page=await browser.newPage({viewport:{width:880,height:560},reducedMotion:'reduce'});
 const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(()=>{
  window.events={};window.calls=[];window.resourceSnapshot={schema_version:1,revision:1,profiles:[]};window.failRead=false;
  window.__TAURI__={core:{invoke:async(cmd,args)=>{
   calls.push({cmd,args});
   if(cmd==='get_boot_args')return {demo:false,expand:false,route:'tasks'};
   if(cmd==='get_settings')return {appearance:'light',shell_mode:'island',dock_edge:'top',sample_interval:2,idle_sample_interval:5,disabled_agents:[],notification_policy:'standard'};
   if(cmd==='get_engine_state')return {snapshots:[],grand_total:{tokens24h:0,tokens:0,cost:0},latest_event:null};
   if(cmd==='window_is_visible')return true;
   if(cmd==='drain_navigation')return [];
   if(cmd==='tasks_attention_summary')return {revision:1,total:0,first:null};
   if(cmd==='tasks_snapshot')return {revision:1,projects:[],tasks:[],runs:[],attentions:[],artifacts:[]};
   if(cmd==='task_sources')return [];
   if(cmd==='chief_resources_snapshot'){if(window.failRead)throw {code:'read_failed',message:'读取失败'};if(window.holdRead)return new Promise(resolve=>{window.holdRead=false;window.heldRead=resolve;});return structuredClone(resourceSnapshot);}
   if(cmd==='chief_resources_save'||cmd==='chief_resources_delete') {
    if(window.holdWrite)await new Promise(resolve=>{window.holdWrite=false;window.heldWrite=resolve;});
    if(window.failWrite){window.failWrite=false;throw {code:'invalid',message:'配置不相容'};}
    if(args.expectedRevision!==resourceSnapshot.revision)throw {code:'stale_revision',message:'版本冲突'};
    if(cmd==='chief_resources_save'){const profile=structuredClone(args.profile);if(!profile.id)profile.id='profile-1';resourceSnapshot.profiles=resourceSnapshot.profiles.filter(p=>p.id!==profile.id);resourceSnapshot.profiles.push(profile);}
    else resourceSnapshot.profiles=resourceSnapshot.profiles.filter(p=>p.id!==args.id);
    resourceSnapshot.revision++;return structuredClone(resourceSnapshot);
   }
   return null;
  }},event:{listen:async(name,fn)=>{(events[name]??=[]).push(fn);return ()=>{};}}};
 });
 await page.goto(`http://127.0.0.1:${server.address().port}/app/ui/index.html?shell=workbench`);
 await page.waitForSelector('[data-tasks-root][data-page-ready=true]');
 await page.getByRole('button',{name:'团队资源配置'}).click();
 await page.getByText('已读取本机配置。').waitFor();
 assert(await page.getByText(/不授予记忆访问权限/).isVisible());
 await page.getByRole('button',{name:'添加配置'}).click();
 for(const [label,value] of Object.entries({'配置名称':'审查组 <x>','角色名称':'审查员','角色所需能力':'read','声明的角色记忆范围':'repo','运行时名称':'桌面','运行时支持的能力':'read','运行时并发上限':'2','模型名称':'模型','模型支持的能力':'read','使用的记忆范围':'repo'})) await page.getByLabel(label,{exact:true}).fill(value);
 await page.getByRole('button',{name:'保存配置'}).click();
 await page.getByText('已保存到本机。').waitFor();
 assert(await page.getByText('审查组 <x>',{exact:true}).isVisible());
 assert(await page.getByRole('button',{name:'编辑',exact:true}).evaluate(el=>el===document.activeElement),'successful save restores focus to the saved entry');
 assert.equal(await page.locator('x').count(),0,'user text must be escaped');
 const saved=await page.evaluate(()=>calls.find(c=>c.cmd==='chief_resources_save'));
 assert.equal(saved.args.expectedRevision,1);assert.deepEqual(saved.args.profile.role.required_capabilities,['read']);
 for(const name of ['编辑','删除']) {
  await page.getByRole('button',{name,exact:true}).focus();
  await page.evaluate(()=>{for(const fn of events['chief-resources://changed']??[])fn({payload:{}});});
  await page.waitForTimeout(50);
  assert(await page.getByRole('button',{name,exact:true}).evaluate(el=>el===document.activeElement),`background refresh keeps focus on ${name}`);
 }
 await page.getByRole('button',{name:'删除',exact:true}).click();
 await page.getByRole('button',{name:'取消',exact:true}).click();
 assert(await page.getByRole('button',{name:'删除',exact:true}).evaluate(el=>el===document.activeElement),'cancel deletion restores the originating button');
 await page.getByRole('button',{name:'编辑',exact:true}).click();
 await page.getByRole('button',{name:'取消编辑'}).click();
 assert(await page.getByRole('button',{name:'编辑',exact:true}).evaluate(el=>el===document.activeElement),'cancel editing restores the originating button');
 await page.getByRole('button',{name:'编辑',exact:true}).click();
 await page.getByLabel('配置名称',{exact:true}).fill('失败草稿');
 await page.evaluate(()=>window.failWrite=true);
 await page.getByRole('button',{name:'保存配置'}).click();
 await page.getByText('配置不相容',{exact:true}).waitFor();
 assert(await page.getByRole('button',{name:'保存配置'}).evaluate(el=>el===document.activeElement),'other failures restore the usable save action');
 assert.equal(await page.getByLabel('配置名称',{exact:true}).inputValue(),'失败草稿');
 await page.getByRole('button',{name:'取消编辑'}).click();
 await page.getByRole('button',{name:'编辑',exact:true}).click();
 await page.getByLabel('配置名称',{exact:true}).fill('保留草稿');
 await page.evaluate(()=>{resourceSnapshot.revision++;resourceSnapshot.profiles[0].name='后台更新';resourceSnapshot.profiles[0].runtime.capabilities=['新能力'];for(const fn of events['chief-resources://changed']??[])fn({payload:{}});});
 await page.getByText(/草稿已保留/).waitFor();
 assert.equal(await page.getByLabel('配置名称',{exact:true}).inputValue(),'保留草稿');
 assert(await page.getByRole('button',{name:'保存配置'}).isDisabled());
 assert(await page.getByText('后台更新',{exact:true}).isVisible());
 assert(await page.getByText(/新能力/).isVisible(),'the latest full declaration must be reviewable before rebasing');
 await page.getByRole('button',{name:'返回任务'}).click();
 await page.getByRole('button',{name:'团队资源配置'}).click();
 assert.equal(await page.getByLabel('配置名称',{exact:true}).inputValue(),'保留草稿');
 await page.getByRole('button',{name:'使用最新版本继续编辑'}).click();
 await page.getByRole('button',{name:'保存配置'}).click();
 await page.getByText('已保存到本机。').waitFor();
 assert.equal((await page.evaluate(()=>calls.filter(c=>c.cmd==='chief_resources_save'))).at(-1).args.expectedRevision,3);
 // The server remains the final authority even when a change event was missed.
 await page.getByRole('button',{name:'编辑',exact:true}).click();
 await page.getByLabel('配置名称',{exact:true}).fill('冲突草稿');
 await page.evaluate(()=>{resourceSnapshot.revision++;resourceSnapshot.profiles[0].name='另一版本';});
 await page.getByRole('button',{name:'保存配置'}).click();
 await page.getByText(/请刷新并核对最新配置/).waitFor();
 assert(await page.getByRole('button',{name:'刷新配置'}).evaluate(el=>el===document.activeElement),'a stale write moves focus to the enabled refresh action');
 assert.equal(await page.getByLabel('配置名称',{exact:true}).inputValue(),'冲突草稿');
 await page.getByRole('button',{name:'刷新配置'}).click();
 await page.getByText('另一版本',{exact:true}).waitFor();
 await page.getByRole('button',{name:'使用最新版本继续编辑'}).focus();
 await page.keyboard.press('Enter');
 assert(await page.getByRole('button',{name:'保存配置'}).evaluate(el=>el===document.activeElement),'accepting the latest version moves focus to the save action');
 await page.keyboard.press('Enter');
 await page.getByText('冲突草稿',{exact:true}).waitFor();
 // If the user deliberately focuses another part of the app during a write,
 // completion must not pull focus back to the resource panel.
 await page.getByRole('button',{name:'编辑',exact:true}).click();
 await page.getByLabel('配置名称',{exact:true}).fill('异步保存');
 await page.evaluate(()=>{window.holdWrite=true});
 await page.getByRole('button',{name:'保存配置'}).click();
 await page.waitForFunction(()=>typeof window.heldWrite==='function');
 await page.locator('.wb-nav-item[data-wb-nav="settings"]').focus();
 await page.evaluate(()=>window.heldWrite());
 await page.getByText('异步保存',{exact:true}).waitFor();
 assert(await page.locator('.wb-nav-item[data-wb-nav="settings"]').evaluate(el=>el===document.activeElement),'save must preserve a user-selected focus outside the panel');
 await page.getByRole('button',{name:'编辑',exact:true}).click();
 await page.getByLabel('配置名称',{exact:true}).fill('离页保存');
 await page.evaluate(()=>{window.holdWrite=true;window.heldWrite=null;});
 await page.getByRole('button',{name:'保存配置'}).click();
 await page.waitForFunction(()=>typeof window.heldWrite==='function');
 await page.locator('.wb-nav-item[data-wb-nav="settings"]').click();
 await page.evaluate(()=>window.heldWrite());
 await page.waitForFunction(()=>resourceSnapshot.profiles[0]?.name==='离页保存');
 assert(await page.locator('.wb-nav-item[data-wb-nav="settings"]').evaluate(el=>el===document.activeElement),'save completion must not focus a cached page');
 await page.locator('.wb-nav-item[data-wb-nav="tasks"]').click();
 await page.getByText('离页保存',{exact:true}).waitFor();
 await page.getByRole('button',{name:'删除',exact:true}).click();
 await page.getByRole('button',{name:'确认删除'}).click();
 await page.getByText('还没有团队资源配置。').waitFor();
 assert(await page.getByRole('button',{name:'添加配置'}).evaluate(el=>el===document.activeElement),'successful deletion restores a visible action');
 await page.evaluate(()=>{window.oldSnapshot=structuredClone(resourceSnapshot);window.holdRead=true;});
 await page.getByRole('button',{name:'刷新配置'}).click();
 await page.waitForFunction(()=>typeof window.heldRead==='function');
 await page.locator('.wb-nav-item[data-wb-nav="settings"]').click();
 await page.evaluate(()=>{const profile=structuredClone(calls.find(c=>c.cmd==='chief_resources_save').args.profile);profile.id='late-profile';profile.name='返回后的最新配置';resourceSnapshot.profiles=[profile];resourceSnapshot.revision++;});
 await page.locator('.wb-nav-item[data-wb-nav="tasks"]').click();
 await page.getByText('返回后的最新配置',{exact:true}).waitFor();
 await page.evaluate(()=>window.heldRead(window.oldSnapshot));
 await page.waitForTimeout(50);
 assert(await page.getByText('返回后的最新配置',{exact:true}).isVisible(),'a read started before leaving cannot replace the restored page');
 await page.getByRole('button',{name:'编辑',exact:true}).click();
 await page.getByLabel('配置名称',{exact:true}).fill('读取失败时的草稿');
 await page.evaluate(()=>{window.failRead=true});
 await page.getByRole('button',{name:'刷新配置'}).click();
 await page.getByText('读取失败',{exact:true}).waitFor();
 assert(await page.getByRole('button',{name:'添加配置'}).isDisabled());
 assert(await page.getByRole('button',{name:'保存配置'}).isDisabled());
 assert.equal(await page.getByLabel('配置名称',{exact:true}).inputValue(),'读取失败时的草稿');
 assert.deepEqual(errors,[]);
 await page.close();
 console.log('PASS: task resource panel, CRUD, declared scope, conflict draft, navigation, escaping and failed read');
} finally {await browser.close();await new Promise(resolve=>server.close(resolve));}
