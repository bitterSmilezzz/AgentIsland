import { invoke } from './tauri.js';
const formats={md:'Markdown',csv:'CSV'};
// Request ownership also covers repeated requests for the same format and clipboard completion.
export function createReportController(read,copy,changed=()=>{}){
  let generation=0,copyRequest=0;
  const state={text:'',format:null,requestedFormat:null,generating:false,copying:false,status:'选择格式生成报告。'};
  const notify=()=>changed({...state,copyable:!!state.text&&!state.generating&&!state.copying});
  return {get state(){return {...state,copyable:!!state.text&&!state.generating&&!state.copying};},
    async generate(format){
      if(!Object.hasOwn(formats,format))return;
      const request=++generation;++copyRequest;
      Object.assign(state,{requestedFormat:format,generating:true,copying:false,status:`正在生成 ${formats[format]} 报告`});notify();
      try{const text=await read(format);if(request!==generation)return;if(typeof text!=='string')throw new Error('invalid report');Object.assign(state,{text,format,requestedFormat:format,status:text?`${formats[format]} 报告已生成`:'暂无报告内容'});}
      catch{if(request!==generation)return;state.requestedFormat=state.format??format;state.status=state.text?'生成失败，保留上次报告。可重试或复制已有内容。':'生成失败，请重试。';}
      finally{if(request===generation){state.generating=false;notify();}}
    },
    async copy(){
      if(!state.text||state.generating||state.copying)return;
      const version=generation,request=++copyRequest,text=state.text;state.copying=true;state.status='正在复制报告';notify();
      try{await copy(text);if(version===generation&&request===copyRequest)state.status='已复制报告';}
      catch{if(version===generation&&request===copyRequest)state.status='无法复制，请在下方手动选中报告。';}
      finally{if(version===generation&&request===copyRequest){state.copying=false;notify();}}
    }
  };
}
export function reportPanelHtml(){return `<div class="sb-page" data-report-panel>
  <div class="wb-report-actions"><div class="wb-format-group" role="group" aria-label="报告格式"><button type="button" class="mini-btn" data-report-format="md" aria-pressed="false">Markdown</button><button type="button" class="mini-btn" data-report-format="csv" aria-pressed="false">CSV</button></div><button type="button" class="mini-btn" data-report-copy disabled>复制报告</button></div>
  <p class="wb-report-status" data-report-status role="status" aria-live="polite">选择格式生成报告。</p>
  <pre class="wb-report-body is-placeholder" data-report-text tabindex="0" aria-label="用量报告正文">生成后可复制，或选中内容。</pre>
</div>`;}
const controllers=new WeakMap();
export function bindReport(panel){
  if(controllers.has(panel))return controllers.get(panel);
  const box=panel.querySelector('[data-report-text]'),status=panel.querySelector('[data-report-status]'),copyButton=panel.querySelector('[data-report-copy]');
  const controller=createReportController(format=>invoke('report_text',{format}),text=>navigator.clipboard.writeText(text),state=>{
    panel.setAttribute('aria-busy',String(state.generating));
    status.textContent=state.status;copyButton.disabled=!state.copyable;
    panel.querySelectorAll('[data-report-format]').forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.reportFormat===state.requestedFormat)));
    const text=state.text||'生成后可复制，或选中内容。';if(box.textContent!==text)box.textContent=text;
    box.classList.toggle('is-placeholder',!state.text);box.setAttribute('aria-label',state.format?`${formats[state.format]} 用量报告正文`:'用量报告正文');
  });
  controllers.set(panel,controller);
  panel.querySelectorAll('[data-report-format]').forEach(button=>button.onclick=()=>controller.generate(button.dataset.reportFormat));
  copyButton.onclick=async()=>{const focused=document.activeElement===copyButton;await controller.copy();if(focused&&panel.isConnected&&document.activeElement===document.body&&!copyButton.disabled)copyButton.focus({preventScroll:true});};return controller;
}
export function visibleReportPanel(){return [...document.querySelectorAll('[data-report-panel]')].find(panel=>!panel.closest('[data-page-outgoing]'));}
export function openUsageReport(root=document){const detail=root.querySelector('[data-usage-report]');if(!detail)return;detail.open=true;detail.querySelector('[data-report-format]')?.focus({preventScroll:true});detail.scrollIntoView({block:'nearest',behavior:'instant'});}
