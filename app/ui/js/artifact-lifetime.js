// A viewed capture keeps the original cache deadline; revalidation never renews it.
export function watchArtifactLifetime({viewer,body,notice,validForMs,validate,current}) {
  let disposed=false,pollTimer,expiryTimer,reading=false;
  const deadline=performance.now()+Math.max(0,Number(validForMs)||0);
  const dispose=()=>{disposed=true;clearTimeout(pollTimer);clearTimeout(expiryTimer);window.removeEventListener('claude-plan:changed',changed);document.removeEventListener('visibilitychange',visible);};
  const invalidate=text=>{if(disposed)return;const focused=body.contains(document.activeElement);body.textContent='';body.hidden=true;notice.textContent=text;if(focused)viewer.querySelector('[data-artifact-close]')?.focus({preventScroll:true});dispose();};
  const changed=()=>invalidate('采集状态已变化，请重新查看来源。');
  const visible=()=>{if(!document.hidden)check();};
  async function check(){
    if(disposed||reading)return;
    if(!viewer.isConnected||!current()){invalidate('查看已结束，请重新读取来源。');return;}
    if(performance.now()>=deadline){invalidate('临时正文已过期，请打开来源。');return;}
    if(document.hidden)return;
    reading=true;
    try{const result=await validate();if(disposed)return;if(!current()||!viewer.isConnected){invalidate('查看已结束，请重新读取来源。');return;}if(!result.available)invalidate(result.notice||'正文已不可用，请打开来源。');}
    catch{invalidate('正文状态无法核实，请重新查看来源。');}
    finally{reading=false;}
  }
  const poll=async()=>{await check();if(!disposed)pollTimer=setTimeout(poll,5000);};
  window.addEventListener('claude-plan:changed',changed);
  document.addEventListener('visibilitychange',visible);
  expiryTimer=setTimeout(()=>invalidate('临时正文已过期，请打开来源。'),Math.max(0,deadline-performance.now()));
  pollTimer=setTimeout(poll,5000);
  return dispose;
}
