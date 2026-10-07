// Only known navigation targets; searching never reads task bodies or invokes IPC.
const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
export function navigationMatches(items,query){
 const words=String(query??'').trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
 return items.filter(item=>words.every(word=>`${item.label} ${item.key} ${item.aliases??''}`.toLocaleLowerCase().includes(word)));
}
export function navigationOptions(items,selected,current){return items.map((item,index)=>`<div role="option" id="quick-route-${index}" aria-selected="${index===selected}" data-quick-index="${index}"><span>${esc(item.label)}</span><small>${item.key===current?'当前页面':'打开'}</small></div>`).join('');}
let controller=null;
export function bindQuickNavigation({items,current,navigate}){
 if(controller){controller.options={items,current,navigate};return controller;}
 const dialog=document.createElement('dialog');dialog.className='quick-navigation';dialog.setAttribute('aria-label','快捷导航');
 dialog.innerHTML='<div class="quick-navigation-search"><label for="quick-navigation-input">前往</label><input id="quick-navigation-input" type="search" autocomplete="off" spellcheck="false" role="combobox" aria-label="搜索功能" aria-expanded="true" aria-controls="quick-navigation-results" aria-autocomplete="list"><button type="button" class="mini-btn" aria-label="关闭快捷导航">Esc</button></div><div id="quick-navigation-results" role="listbox" aria-label="功能页面"></div><p class="quick-navigation-hint">↑ ↓ 选择 · 回车打开</p>';
 document.body.appendChild(dialog);
 const input=dialog.querySelector('input'),results=dialog.querySelector('[role=listbox]');let found=[],selected=0,caller=null;
 const state={options:{items,current,navigate},open};controller=state;
 function render(){found=navigationMatches(state.options.items,input.value);selected=Math.min(selected,Math.max(0,found.length-1));results.innerHTML=found.length?navigationOptions(found,selected,state.options.current()):'<p class="quick-navigation-empty">没有匹配的功能</p>';if(found.length)input.setAttribute('aria-activedescendant',`quick-route-${selected}`);else input.removeAttribute('aria-activedescendant');results.querySelector('[aria-selected=true]')?.scrollIntoView({block:'nearest'});}
 function close(){dialog.close();}
 function choose(){const item=found[selected];if(!item)return;const same=item.key===state.options.current();if(!same)caller=null;close();if(!same)state.options.navigate(item.key);}
 function open(){
  if(!document.querySelector('.wb'))return;
  // Keep confirmations authoritative; do not open a second modal over a pending write.
  const gate=[...document.querySelectorAll('[data-confirm],[data-mcp-confirm],[data-prompt-confirm],[data-layout-confirm],dialog[open]')].find(node=>node!==dialog&&!node.hidden&&node.getClientRects().length);
  if(gate){gate.querySelector('button:not(:disabled)')?.focus({preventScroll:true});return;}
  if(dialog.open){input.focus();return;}
  caller=document.activeElement;input.value='';selected=0;dialog.showModal();render();input.focus({preventScroll:true});
 }
 dialog.addEventListener('close',()=>{input.value='';results.replaceChildren();input.removeAttribute('aria-activedescendant');if(caller?.isConnected)caller.focus({preventScroll:true});caller=null;});
 dialog.querySelector('button').onclick=close;
 dialog.addEventListener('click',event=>{if(event.target===dialog){const r=dialog.getBoundingClientRect();if(event.clientX<r.left||event.clientX>r.right||event.clientY<r.top||event.clientY>r.bottom)close();}});
 input.oninput=()=>{selected=0;render();};
 input.onkeydown=event=>{if(event.isComposing)return;if(event.key==='Escape'){event.preventDefault();close();}else if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();if(found.length){selected=(selected+(event.key==='ArrowDown'?1:-1)+found.length)%found.length;render();}}else if(event.key==='Enter'){event.preventDefault();choose();}};
 results.onclick=event=>{const row=event.target.closest('[data-quick-index]');if(row){selected=Number(row.dataset.quickIndex);choose();}};
 document.addEventListener('keydown',event=>{if((event.metaKey||event.ctrlKey)&&!event.altKey&&!event.shiftKey&&event.key.toLowerCase()==='k'&&!event.isComposing){if(!document.querySelector('.wb'))return;event.preventDefault();open();}});
 return state;
}
