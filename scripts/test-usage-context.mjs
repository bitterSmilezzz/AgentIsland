import assert from 'node:assert/strict';
import { usageContextHtml } from '../app/ui/js/usage-context.js';
const row = {agent_id:'codex',agent_name:'Codex',provider:'recorded-provider',requested_model:'requested-model',tokens:600};
const report = {matched_tokens:600,unmatched_tokens:400,other_matched_tokens:0,rows:[row]};
const html = usageContextHtml(report,1000);
assert.match(html,/上下文可核对 60%/);
assert.match(html,/未核对 <strong>400<\/strong>/);
assert.match(html,/会话服务/);assert.match(html,/请求模型/);
assert.match(html,/不代表接口、账户或账单/);
assert.match(html,/接口身份未记录/);
assert.doesNotMatch(html,/<details[^>]* open/,'source detail starts collapsed');
const almost = {...report,matched_tokens:999,unmatched_tokens:1,rows:[{...row,tokens:999}]};
assert.match(usageContextHtml(almost,1000),/99.9%/,'unknown remainder must not round to full coverage');
const zero={matched_tokens:0,unmatched_tokens:0,other_matched_tokens:0,rows:[]};
assert.match(usageContextHtml(zero,0),/暂无用量/);
assert.doesNotMatch(usageContextHtml(undefined,0),/暂无用量/,'missing context is not verified zero');
for(const bad of [undefined,{...report,matched_tokens:1001},{...report,unmatched_tokens:-1},{...report,rows:[{...row,tokens:500}]},{...report,rows:Array(33).fill(row)}]) {
  assert.match(usageContextHtml(bad,1000),/暂不可核对/);
}
const hostile={...report,rows:[{...row,agent_name:'<img src=x>',provider:'" onfocus="x',requested_model:'<script>alert(1)</script>'}]};
assert.doesNotMatch(usageContextHtml(hostile,1000),/<img|<script| onfocus="/);
assert.match(usageContextHtml(hostile,1000),/&lt;script&gt;/);
const omitted={...report,other_matched_tokens:100,rows:[{...row,tokens:500}]};
assert.match(usageContextHtml(omitted,1000),/其他已核对上下文 · 100 tokens/);
console.log('PASS: context versus interface, complete coverage accounting, zero versus absent, bounded rows, escaping and collapsed disclosure');
