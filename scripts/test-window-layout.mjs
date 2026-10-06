import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const {pageWindowLayout,layoutPreviewMarkup,layoutHistoryMarkup}=await import('../app/ui/js/window-layout-page.js');
const html=pageWindowLayout();
assert.match(html,/data-layout-apply disabled/);
assert.match(html,/data-layout-permission hidden/);
const snapshot={displays:[{screen_id:'d',rect:{x:-1200,y:24,width:1200,height:800}}],windows:[{window_id:'a',application:'<Tool & title>'}]};
const preview={geometry:{screen_id:'d',placements:[{window_id:'a',target:{x:-1200,y:24,width:590,height:800},restriction:'<unavailable>'}]}};
const markup=layoutPreviewMarkup(preview,snapshot);
assert.match(markup,/left:0%;top:0%/);
assert.match(markup,/&lt;Tool &amp; title&gt;/);
assert.match(markup,/&lt;unavailable&gt;/);
assert.doesNotMatch(markup,/<Tool|<unavailable>/);
assert.match(layoutPreviewMarkup(preview,{displays:[],windows:[]}),/目标屏幕已失效/);
console.log('PASS: window preview uses logical origins, escapes labels and starts with explicit disabled actions');

const history=layoutHistoryMarkup(Array.from({length:8},(_,i)=>({id:'record-'+i,created_ms:i,phase:i?'finished':'pending',slots:[{agent_id:'<unknown-tool>',outcome:i?'failed':null,before:{x:-1200,y:24,width:590,height:800},target:{x:0,y:24,width:590,height:800},actual:null}]})));
assert.match(history,/更多记录 · 3 条/);assert.match(history,/执行读回/);assert.match(history,/未核实/);assert.match(history,/&lt;unknown-tool&gt;/);assert.doesNotMatch(history,/<unknown-tool>/);assert.match(layoutHistoryMarkup([]),/还没有/);assert.match(pageWindowLayout(),/data-layout-history/);
console.log('PASS: durable geometry history is bounded in presentation, escapes identities and distinguishes unknown readback');
