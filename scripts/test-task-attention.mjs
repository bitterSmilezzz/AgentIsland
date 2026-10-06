import assert from 'node:assert/strict';
import { attentionDetail, taskAttentionHtml, taskCoversEvent, taskCoversSnapshot } from '../app/ui/js/task-attention.js';
import { taskIdFromIntent } from '../app/ui/js/navigation.js';
const id='019c6e27-e55b-73d1-87d8-4e01f1f75043';
assert.equal(taskIdFromIntent(`Task(${JSON.stringify(id)})`),id);
assert.equal(taskIdFromIntent('Task("../arbitrary")'),null);
assert.equal(taskIdFromIntent('Tasks'),null);
const item={task_id:id,run_id:'run',title:'整理 <方案>',label:'待回答',agent_id:'codex',target:{exactSession:true,url:`codex://threads/${id}`,hint:'打开对应会话',label:'打开会话'},failure:false,observed:true};
const engine={session_navigation:{codex:{url:item.target.url}},snapshots:[{id:'codex',level:'attention',current_action:'选择发布方式'}]};
assert.equal(attentionDetail(item,engine),'选择发布方式');
engine.session_navigation.codex.url='codex://threads/another';
assert.equal(attentionDetail(item,engine),'在来源工具中处理','never use a different conversation question');
assert.equal(attentionDetail({...item,detail:'本地方案引用'},engine),'本地方案引用');
const html=taskAttentionHtml({revision:2,total:4,human_count:3,failed_count:1,first:item},engine);
assert.match(html,/4 个需处理/);assert.match(html,/整理 &lt;方案&gt;/);assert.match(html,/data-attention-revision="2"/);assert.match(html,/打开会话/);
assert.doesNotMatch(html,/忽略|本地已处理|批准操作/,'the compact surface does not acknowledge or approve tasks');
assert.equal(taskAttentionHtml({total:0,first:null},engine),'');
assert.match(taskAttentionHtml({error:'损坏 <文件>'},engine),/损坏 &lt;文件&gt;/);
console.log('PASS: compact counts, precise conversation detail, escaped text and navigation identities');

const summary={first:item};
const eventEngine={latest_event:{id:'event',agent_id:'codex',event_type:'attention',externally_delivered:false},event_navigation:{event:{url:item.target.url}}};
assert.equal(taskCoversEvent(summary,eventEngine),true,'the same verified question needs one banner');
eventEngine.latest_event.detail='kind=memory elapsed_ms=1';assert.equal(taskCoversEvent(summary,eventEngine),false,'resource alerts remain separate');
eventEngine.latest_event.detail=null;eventEngine.latest_event.externally_delivered=true;assert.equal(taskCoversEvent(summary,eventEngine),false,'external notices are not silently deduplicated');

assert.equal(taskCoversSnapshot(summary,{id:"codex",level:"attention"},item.target),true);
assert.equal(taskCoversSnapshot(summary,{id:"codex",level:"attention"},{url:"another"}),false);

assert.equal(taskCoversSnapshot({first:{...item,observed:false}},{id:"codex",level:"attention"},item.target),false);
engine.session_navigation.codex.url=item.target.url;
assert.notEqual(attentionDetail({...item,observed:false},engine),"选择发布方式","manual records cannot borrow a different observed question");
