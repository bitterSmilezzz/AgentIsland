import assert from 'node:assert/strict';
import {skillPackagePreviewHtml,skillPackagesHtml,skillPackageRecordsHtml,skillSyncRowsHtml,packageSize} from '../app/ui/js/skills-package-page.js';
const preview={name:'fixture<script>',action:'update',files:2,bytes:220,notice:'Fixture <notice>',change_count:103,changes:[{path:'scripts/<unsafe>.sh',kind:'changed'}]};
const html=skillPackagePreviewHtml(preview);
assert.ok(!html.includes('<script>'));assert.ok(!html.includes('<unsafe>'));assert.match(html,/&lt;unsafe&gt;/);
assert.match(html,/更新/);assert.match(html,/103 项/);assert.match(html,/显示前 1 项/);assert.match(html,/220 B/);
assert.equal(packageSize(0),'0 B');assert.equal(packageSize(2048),'2.0 KiB');assert.equal(packageSize(1048576),'1.00 MiB');
assert.match(skillPackagesHtml(),/data-skill-package-confirm hidden/);assert.match(skillPackagesHtml(),/需在目标客户端核对/);
const trash=skillPackagePreviewHtml({...preview,action:'trash',changes:[{path:'package/SKILL.md',kind:'moved'}]});
assert.match(trash,/恢复记录 · 2 个移出文件/);assert.match(trash,/移入废纸篓/);assert.match(trash,/移出/);assert.ok(!trash.includes('个目标文件'));
console.log('PASS: package preview escapes labels, discloses partial difference list and reports small nonzero sizes');

const records=skillPackageRecordsHtml(Array.from({length:8},(_,i)=>({id:'fixture-'+i,name:'sample<'+i,created_ms:0,state:'inactive',notice:'retained'})));
assert.match(records,/更多安装记录 · 3 条/);assert.match(records,/data-skill-package-trash="7"/);assert.ok(!records.includes('sample<'));assert.match(records,/sample&lt;/);
assert.match(skillPackageRecordsHtml([]),/还没有/);
console.log('PASS: recent five records keep the view compact; older records remain reachable with stable row indices');

const syncRows=skillSyncRowsHtml(Array.from({length:8},(_,i)=>({tool:i%2?'claude':'codex',name:'sample<'+i,notice:'目录候选',available:true})));
assert.match(syncRows,/更多来源 · 3 项/);assert.match(syncRows,/data-skill-sync-preview="7"/);assert.match(syncRows,/Claude Code/);assert.ok(!syncRows.includes('sample<'));
assert.match(skillPackagePreviewHtml({...preview,target:'claude'}),/Claude Code 用户级/);assert.match(skillPackagePreviewHtml({...preview,target:'unknown'}),/目标未核实/);
assert.match(skillPackageRecordsHtml([{id:'one',name:'same',state:'applied',target:'claude',notice:'保留包'}]),/Claude Code · 目录已安装/);
console.log('PASS: sync sources and target-aware previews escape labels and keep old sources reachable');

assert.match(skillPackagePreviewHtml({...preview,target:'claude',source:'codex'}),/同步更新/);

assert.match(syncRows,/data-skill-body-read="7"/);assert.match(syncRows,/编辑正文/);
assert.match(skillPackagePreviewHtml({...preview,target:'claude',action:'edit'}),/保存正文/);
assert.match(skillPackagesHtml(),/技能正文编辑/);
console.log('PASS: body editor is reachable by exact source identity; preview labels the existing target');
