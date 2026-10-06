const HOUR=3600000;
const ranges={'24':{hours:24,label:'24 小时'},'7':{hours:168,label:'7 天'},'30':{hours:720,label:'30 天'}};
const normalize=range=>Object.hasOwn(ranges,range)?String(range):'24';
const esc=value=>String(value).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
export function trendSeries(hourly,range='24',now=Date.now()){
  range=normalize(range);const end=Math.floor(now/HOUR)*HOUR,start=end-(ranges[range].hours-1)*HOUR,values=new Map();
  for(const item of hourly){if(!Array.isArray(item))continue;const [ts,tokens]=item;if(!Number.isFinite(ts)||!Number.isFinite(tokens)||tokens<0||ts%HOUR!==0||ts<start||ts>end)continue;values.set(ts,(values.get(ts)??0)+tokens);}
  const points=[...values].sort((a,b)=>a[0]-b[0]),peak=Math.max(0,...points.map(point=>point[1]));
  return {range,label:ranges[range].label,start,end,points,peak};
}
export function trendPlot(series,width=270,height=96){
  const scale=Math.max(1,series.peak);
  const points=series.points.map(([ts,value])=>({ts,value,x:4+(ts-series.start)/(series.end-series.start)*(width-8),y:height-16-value/scale*(height-30)}));
  const path=points.map((p,index)=>`${!index||p.ts-points[index-1].ts>HOUR?'M':'L'}${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(' ');
  return {width,height,points,path,peak:points.find(p=>p.value===series.peak)};
}
export function trendHtml(series){
  const plot=trendPlot(series,1000,200),format=ts=>new Date(ts).toLocaleString(undefined,{month:'numeric',day:'numeric',hour:'2-digit',minute:'2-digit'});
  return `<svg class="usage-trend" viewBox="0 0 ${plot.width} ${plot.height}" role="img" aria-label="${esc(series.label)}范围的小时记录趋势，${series.points.length} 个记录桶，峰值 ${series.peak} tokens">${[1,2,3].map(i=>`<line x1="4" x2="${plot.width-4}" y1="${(plot.height-16)*i/4}" y2="${(plot.height-16)*i/4}" stroke="var(--hairline)" stroke-width="0.5"/>`).join('')}${plot.path?`<path d="${plot.path}" fill="none" stroke="var(--cyan)" stroke-width="1.4" vector-effect="non-scaling-stroke"/>${plot.points.filter((p,i)=>!i||p.ts-plot.points[i-1].ts>HOUR||i===plot.points.length-1||plot.points[i+1].ts-p.ts>HOUR).map(p=>`<circle cx="${p.x}" cy="${p.y}" r="4" fill="var(--cyan)"/>`).join('')}${plot.peak?`<circle cx="${plot.peak.x}" cy="${plot.peak.y}" r="6" fill="var(--cyan)"/>`:''}`:''}</svg><div class="usage-trend-axis"><span>${esc(format(series.start))}</span><span>${esc(format(series.end))}</span></div><p class="usage-trend-note" role="status" aria-live="polite">${series.points.length?`${series.label} · ${series.points.length} 个小时记录 · 峰值 ${series.peak.toLocaleString()} tokens`:'此范围暂无小时记录'}。空缺不代表零消耗。</p>`;
}
export function trendCardHtml(){return `<div class="card-box usage-trend" data-usage-trend><div class="usage-trend-heading"><h4>使用趋势</h4><div class="usage-trend-ranges" role="group" aria-label="趋势范围">${['24','7','30'].map(range=>{const item=ranges[range];return`<button type="button" data-usage-range="${range}" aria-pressed="${range==='24'}">${item.label}</button>`;}).join('')}</div></div><div data-usage-trend-plot></div><p class="usage-trend-scope">切换仅影响趋势，汇总与模型明细为 24h。</p></div>`;}
export function bindTrend(card,hourly,range='24',changed=()=>{},now=Date.now()){
  const plot=card.querySelector('[data-usage-trend-plot]');
  const paint=value=>{range=normalize(value);card.querySelectorAll('[data-usage-range]').forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.usageRange===range)));plot.innerHTML=trendHtml(trendSeries(hourly,range,now));changed(range);};
  card.querySelectorAll('[data-usage-range]').forEach(button=>{
    button.onclick=()=>paint(button.dataset.usageRange);
    button.onkeydown=event=>{if(event.isComposing||event.metaKey||event.ctrlKey||event.altKey||event.shiftKey||!['ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;event.preventDefault();const buttons=[...card.querySelectorAll('[data-usage-range]')],index=buttons.indexOf(button),next=event.key==='Home'?0:event.key==='End'?buttons.length-1:(index+(event.key==='ArrowRight'?1:-1)+buttons.length)%buttons.length;paint(buttons[next].dataset.usageRange);buttons[next].focus({preventScroll:true});};
  });paint(range);
}
export function hourRecords(hourly,now=Date.now()){
  const series=trendSeries(hourly,'24',now),values=new Map(series.points);
  return Array.from({length:24},(_,i)=>{const ts=series.start+i*HOUR;return {ts,tokens:values.has(ts)?values.get(ts):null};});
}
