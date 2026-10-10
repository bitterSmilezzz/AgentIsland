import assert from 'node:assert/strict';
import { taskGroup, runSourceAction, renderTaskHistory, artifactReadAction, taskGateNotice } from '../app/ui/js/tasks-page.js';
const data = { runs: [], attentions: [] };
const task = { current_run_id: null };
assert.equal(taskGroup(data, task), 'queued');
const run = {id:'run',status:'running'};
data.runs.push(run);task.current_run_id='run';
assert.equal(taskGroup(data, task), 'running');
run.status='ready';assert.equal(taskGroup(data, task),'finished','ready is finished execution, not accepted');
data.attentions.push({run_id:'run',state:'open'});
assert.equal(taskGroup(data,task),'attention','an open review takes priority over ready');
data.attentions[0].state='resolved';assert.equal(taskGroup(data,task),'finished');
for (const status of ['failed','waiting']) {run.status=status;assert.equal(taskGroup(data,task),'attention');}
for (const status of ['cancelled','accepted']) {run.status=status;assert.equal(taskGroup(data,task),'finished');}
run.status='running';data.attentions.push({run_id:'old-run',state:'open'});
assert.equal(taskGroup(data,task),'running','old-run gates cannot affect the current run');
console.log('PASS: task grouping, open review precedence and historical gate isolation');

const historical = { id: 'old', task_id: 'task', status: 'ready', started_ms: 0, source: { agent_id: 'codex', thread_id: 'old-thread' } };
assert.deepEqual(runSourceAction(historical), { runId: 'old', label: '打开来源会话' });
assert.equal(runSourceAction({ ...historical, source: null }), null, 'old local records never inherit the task source');
assert.equal(runSourceAction({ ...historical, source: { agent_id: 'traework' } }).label, '打开来源工具');
const historyData = { runs: [historical, { ...historical, id: 'local', source: null }, { ...historical, id: 'foreign', task_id: 'other' }], artifacts: [{ id: 'artifact', run_id: 'old', title: '<img src=x>' }], attentions: [] };
const html = renderTaskHistory(historyData, 'task');
assert.match(html, /运行记录 · 2/);
assert.match(html, /data-task-run-open="old"/);
assert.match(html, /未保存来源/);
assert.ok(!html.includes('data-task-run-open="local"') && !html.includes('foreign'));
assert.ok(html.includes('&lt;img src=x&gt;') && !html.includes('<img src=x>'));
console.log('PASS: historical source ownership, local fallback refusal and escaped output');

assert.equal(artifactReadAction({id:'manual',event_ref:null},'old'),null,'local title is not a source content capability');
const sourceArtifact={id:'source',run_id:'old',kind:'answer',title:'来源问题',event_ref:'a'.repeat(64)};
assert.deepEqual(artifactReadAction(sourceArtifact,'old'),{artifactId:'source',runId:'old',label:'查看问题'});
const sourceHtml=renderTaskHistory({...historyData,artifacts:[sourceArtifact]},'task');
assert.match(sourceHtml,/data-task-artifact-read="source"/);assert.match(sourceHtml,/data-artifact-run="old"/);
console.log('PASS: source content actions retain exact artifact and run ownership');

assert.equal(artifactReadAction({...sourceArtifact,kind:"plan_approval"},"old").label,"查看方案");

for (const kind of ['answer', 'plan_approval', 'result_review']) {
  assert.equal(taskGateNotice({kind, observed:false}), '本地记录，处理后标记即可。');
  assert.ok(!taskGateNotice({kind}).includes('来源'), 'legacy missing provenance cannot claim a source gate');
}
assert.match(taskGateNotice({kind:'plan_approval',observed:true}), /未提供方案正文/);
assert.match(taskGateNotice({kind:'answer',observed:true}), /来源工具/);
console.log('PASS: manual and legacy gates do not claim source approval or unavailable tool content');
