import assert from 'node:assert/strict';
import {navigationMatches,navigationOptions,bindQuickNavigation} from '../app/ui/js/quick-navigation.js';
const items=[{key:'provider',label:'模型与连接',aliases:'MCP Skills 提示词'},{key:'tasks',label:'任务'},{key:'settings',label:'设置'}];
assert.deepEqual(navigationMatches(items,' mCp ').map(x=>x.key),['provider']);
assert.deepEqual(navigationMatches(items,'Skills 提示词').map(x=>x.key),['provider']);
assert.equal(navigationMatches(items,'不存在').length,0);
assert.equal(navigationMatches(items,'').length,3);
assert.match(navigationOptions([{key:'<bad>',label:'<script>'}],0,'<bad>'),/&lt;script&gt;/);
assert.match(navigationOptions(items,1,'tasks'),/aria-selected="true"[^>]*data-quick-index="1"/);
assert.match(navigationOptions(items,1,'tasks'),/当前页面/);
console.log('PASS: navigation search matches bounded known targets, escapes labels, and marks selection/current page');

// Exercise the controller's focus policy, including shortcuts from a toolbar
// outside the preview. Browser acceptance separately checks the real DOM/IPC.
const listeners=new Map();
const element=(name)=>({
 name,hidden:false,children:[],attributes:new Map(),handlers:new Map(),
 setAttribute(key,value){this.attributes.set(key,value);},
 removeAttribute(key){this.attributes.delete(key);},
 hasAttribute(key){return this.attributes.has(key);},
 get tabIndex(){return this.attributes.get('tabindex');},
 set tabIndex(value){this.attributes.set('tabindex',String(value));},
 addEventListener(key,handler){this.handlers.set(key,handler);},
 focus(){document.activeElement=this;},
 contains(node){return node===this||this.children.includes(node);},
 getClientRects(){return this.collapsed?[]:[{}];},
 replaceChildren(){},scrollIntoView(){},
});
const input=element('search'),results=element('results'),close=element('close');
results.querySelector=()=>null;
const dialog=element('navigation');dialog.open=false;
dialog.querySelector=selector=>selector==='input'?input:selector==='button'?close:results;
dialog.showModal=()=>{dialog.open=true;};
dialog.close=()=>{dialog.open=false;dialog.handlers.get('close')?.();};
let gates=[];
const toolbar=element('toolbar'),cancel=element('cancel'),confirm=element('confirm');
globalThis.document={
 activeElement:toolbar,body:{appendChild(){}},
 createElement:()=>dialog,
 querySelector:selector=>selector==='.wb'?{}:null,
 querySelectorAll:selector=>gates.filter(gate=>selector.split(',').some(part=>part===gate.selector)),
 addEventListener:(name,handler)=>listeners.set(name,handler),
};
const quick=bindQuickNavigation({items,current:()=> 'provider',navigate(){throw new Error('pending write must not navigate');}});
const shortcut=()=>{
 let prevented=false;
 listeners.get('keydown')({ctrlKey:true,key:'k',preventDefault(){prevented=true;}});
 assert.equal(prevented,true);
};
for(const selector of ['[data-confirm]','[data-mcp-confirm]','[data-prompt-confirm]','[data-claude-confirm]','[data-layout-confirm]','dialog[open]']){
 const gate=element(selector);gate.selector=selector;gate.children=[cancel,confirm];gates=[gate];
 cancel.focus();shortcut();
 assert.equal(document.activeElement,cancel,`${selector}: keep the existing cancel choice`);
 assert.equal(dialog.open,false);
 confirm.focus();shortcut();
 assert.equal(document.activeElement,confirm,`${selector}: retain an intentional focused action`);
 toolbar.focus();quick.open();
 assert.equal(document.activeElement,gate,`${selector}: toolbar returns to the preview itself`);
 assert.equal(gate.tabIndex,'-1','preview can receive focus without an extra tab stop');
 assert.equal(dialog.open,false);
}
gates[0].hidden=true;toolbar.focus();quick.open();
assert.equal(dialog.open,true,'hidden confirmations must not block navigation');dialog.close();
gates[0].hidden=false;gates[0].collapsed=true;quick.open();
assert.equal(dialog.open,true,'collapsed confirmations must not block navigation');dialog.close();
gates=[];shortcut();assert.equal(dialog.open,true,'navigation resumes after confirmation is dismissed');
assert.equal(document.activeElement,input);dialog.close();
assert.equal(listeners.size,1,'one keyboard listener for the controller');
console.log('PASS: all pending previews retain focus, toolbar never selects a write, hidden previews allow navigation');
