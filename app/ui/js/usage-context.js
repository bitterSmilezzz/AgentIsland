const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const count = value => Number.isSafeInteger(value) && value >= 0;
export function usageContextHtml(report, total, format = String) {
  const valid = report && count(total) && count(report.matched_tokens) && count(report.unmatched_tokens)
    && count(report.other_matched_tokens) && Array.isArray(report.rows) && report.rows.length <= 32
    && report.rows.every(row => count(row.tokens))
    && report.matched_tokens + report.unmatched_tokens === total
    && report.rows.reduce((n,row) => n + row.tokens, report.other_matched_tokens) === report.matched_tokens;
  const summary = !valid ? '暂不可核对' : total === 0 ? '暂无用量' : `上下文可核对 ${Math.min(report.unmatched_tokens > 0 ? 99.9 : 100, Math.floor(1000 * report.matched_tokens / total)/10)}%`;
  return `<details class="usage-context" data-usage-context><summary>用量来源<span>${summary}</span></summary>
    <div class="usage-context-body"><p>会话记录标识，不代表接口、账户或账单。</p>
      ${!valid ? '<p role="status">当前快照未提供可核对的上下文。</p>' : `
      <div class="usage-context-coverage"><span title="${report.matched_tokens} tokens">已核对 <strong>${esc(format(report.matched_tokens))}</strong></span><span title="${report.unmatched_tokens} tokens">未核对 <strong>${esc(format(report.unmatched_tokens))}</strong></span><small>24h 净 tokens</small></div>
      ${report.rows.length ? `<div class="usage-context-scroll" tabindex="0" role="region" aria-label="会话上下文明细"><table><thead><tr><th scope="col">工具</th><th scope="col">会话服务</th><th scope="col">请求模型</th><th scope="col">净 tokens</th></tr></thead><tbody>${report.rows.map(row => `<tr><td>${esc(row.agent_name)}</td><td>${esc(row.provider ?? '未记录')}</td><td>${esc(row.requested_model ?? '未记录')}</td><td title="${row.tokens}">${esc(format(row.tokens))}</td></tr>`).join('')}</tbody></table></div>` : ''}
      ${report.other_matched_tokens > 0 ? `<p>其他已核对上下文 · ${esc(format(report.other_matched_tokens))} tokens</p>` : ''}
      <p class="usage-context-limit">${total === 0 ? '本次快照没有用量记录。' : '接口身份未记录，暂不能按接口核算。'}</p>`}
    </div></details>`;
}
const bound = new WeakSet();
export function bindUsageContext(body) {
  const detail = body.querySelector('[data-usage-context]');
  if (!detail) return;
  detail.open = body.dataset.usageContextOpen === 'true';
  if (bound.has(body)) return;
  bound.add(body);
  body.addEventListener('toggle', event => {
    if (event.target.matches('[data-usage-context]')) body.dataset.usageContextOpen = String(event.target.open);
  }, true);
}
