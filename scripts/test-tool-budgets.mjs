import assert from 'node:assert/strict';
import { createToolBudgetController } from '../app/ui/js/tool-budget-state.js';
import { toolBudgetsHtml } from '../app/ui/js/tool-budgets.js';
const report = (a = 1000, b = 2000) => ({ alertsEnabled: true, rows: [
  {agentId:'a',name:'A',budget:a,used:null,status:'unavailable',source:'unavailable'},
  {agentId:'b',name:'B',budget:b,used:0,status:'normal',source:'local'},
] });
let current = report(), failRead = false, failWrite = false, hold, calls = [];
const controller = createToolBudgetController(
  async () => { if (failRead) throw 'fixture'; return structuredClone(current); },
  async (id, budget, expected) => {
    calls.push({id,budget,expected});
    if (hold) await hold;
    if (failWrite) throw '预算已在其他窗口修改';
    current.rows.find(row => row.agentId === id).budget = budget;
    return structuredClone(current);
  },
);
assert.equal(controller.state.canSave, false);
await controller.load(); assert.equal(controller.state.selected, 'a');
assert.equal(controller.state.row.used, null, 'unknown source remains unknown');
controller.edit('1500'); controller.select('b');
assert.equal(controller.state.dirty, false); assert.equal(controller.state.hasDraft, true, 'a hidden tool draft keeps the native lease');
assert.equal(controller.state.row.used, 0, 'verified zero remains zero');
controller.edit('2500'); controller.select('a');
assert.equal(controller.state.value, '1500', 'each tool keeps its own draft');
current = report(1200, 2200); await controller.load();
assert.equal(controller.state.value, '1500', 'refresh keeps dirty input');
failWrite = true; await controller.save();
assert.deepEqual(calls.at(-1), {id:'a',budget:1500,expected:1000}, 'refresh cannot silently rebase the compare-and-save');
assert.match(controller.state.status, /其他窗口/);
assert.equal(controller.state.value, '1500');
controller.cancel(); assert.equal(controller.state.value, '1200');
assert.equal(controller.state.canSave, false);
controller.edit('1800'); failWrite = false; await controller.save();
assert.deepEqual(calls.at(-1), {id:'a',budget:1800,expected:1200});
assert.equal(controller.state.dirty, false);
controller.select('b'); assert.equal(controller.state.value, '2500', 'saving A preserves B draft');
for (const invalid of ['', '-1', '1.5', '1e6', '1000000001', 'NaN', ' 2 ', '<script>']) {
  controller.edit(invalid); assert.equal(controller.state.canSave, false, invalid);
}
controller.edit('0'); assert.equal(controller.state.canSave, true);
failRead = true; await controller.load();
assert.equal(controller.state.value, '0'); assert.equal(controller.state.readFailed, true);
assert.equal(controller.state.report.rows.length, 2, 'failed refresh keeps the previous report');
failRead = false; await controller.save();
assert.equal(current.rows[1].budget, 0); assert.match(controller.state.status, /已取消/);
controller.edit('3000');
let release; hold = new Promise(resolve => { release = resolve; });
const saving = controller.save();
controller.select('a'); controller.edit('9000'); controller.cancel(); await controller.load();
assert.equal(controller.state.selected, 'b'); assert.equal(controller.state.value, '3000');
const count = calls.length; await controller.save(); assert.equal(calls.length, count, 'only one in-flight mutation');
release(); await saving; hold = null;
assert.equal(controller.state.busy, false); assert.equal(controller.state.dirty, false);
const lost = createToolBudgetController(async () => report(), async () => null);
await lost.load(); lost.edit('1600'); await lost.save();
assert.equal(lost.state.value, '1600'); assert.equal(lost.state.dirty, true, 'lost response must not discard draft');
assert.match(lost.state.status, /刷新核对/);
assert.match(toolBudgetsHtml(), /aria-live="polite"/);
assert.match(toolBudgetsHtml(), /本地提醒，不限制请求/);
const attention = createToolBudgetController(async () => ({alertsEnabled:true,rows:[
 {agentId:'empty',budget:0,used:null,status:'unset'},
 {agentId:'configured',budget:2000,used:0,status:'normal'},
 {agentId:'warning',budget:1000,used:850,status:'warning'},
]}));
await attention.load();assert.equal(attention.state.selected,'warning','first open prioritizes a budget needing attention');
console.log('PASS: independent drafts, unknown versus zero, optimistic conflict, explicit rebase, persistence failure, limits, removal, late completion and lost response');
