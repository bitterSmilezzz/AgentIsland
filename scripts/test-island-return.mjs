import assert from 'node:assert/strict';
import { stageNavigation, resizeNavigation } from '../app/ui/js/island-navigation.js';
class Animation {
  constructor() { this.finished = new Promise(resolve => setTimeout(resolve, 10)); }
  cancel() {}
}
class Element {
  constructor(width, height) { this.width=width; this.height=height; this.style={}; this.dataset={}; this.children=[]; this.classList={add(){},remove(){}}; }
  getBoundingClientRect() { return {width:this.width,height:this.height}; }
  querySelector(s) { return s === '.card-inner' ? this.inner : null; }
  querySelectorAll() { return []; }
  appendChild(n) { this.children.push(n); }
  remove() { this.removed=true; }
  animate() { return new Animation(); }
}
globalThis.window={innerWidth:330,innerHeight:370};
globalThis.requestAnimationFrame=cb=>setTimeout(cb,2);
globalThis.getComputedStyle=()=>({opacity:'1',transform:'none'});
const card=new Element(300,333); card.inner=new Element(298,331);
const copy=new Element(328,368);
stageNavigation(card,{copy,from:{width:330,height:370},route:'list',previous:'agentDetail:codex'});
let settledViewport;
const calls=[];
const invoke=async (_,size)=>{
  calls.push(size);
  // A native geometry command acknowledges submission before WebKit's resize arrives.
  setTimeout(()=>Object.assign(window,{innerWidth:size.width,innerHeight:size.height}),25);
};
await resizeNavigation(card,invoke,()=>true,()=>{settledViewport={width:window.innerWidth,height:window.innerHeight};});
assert.deepEqual(settledViewport,{width:300,height:333},'return must retain its fixed surface until the native viewport catches up; otherwise sampling can resize again and the surface jumps');
assert.equal(calls.length,2);
console.log('PASS: Agent detail return waits for the final native viewport before releasing geometry');

// A busy WebView can commit the resize after the old 240ms budget.
// Native IPC acknowledgement must not count as successful viewport settlement.
Object.assign(window, {innerWidth:330,innerHeight:370});
const delayed=new Element(300,333); delayed.inner=new Element(298,331);
stageNavigation(delayed,{copy:new Element(328,368),from:{width:330,height:370},route:'list',previous:'settings'});
let delayedSettlement;
await resizeNavigation(delayed,async (_,size)=>{
  if (size.width===300) setTimeout(()=>Object.assign(window,{innerWidth:size.width,innerHeight:size.height}),360);
},()=>true,()=>{delayedSettlement={width:window.innerWidth,height:window.innerHeight};});
assert.deepEqual(delayedSettlement,{width:300,height:333},'a slow viewport commit must not release return geometry at an arbitrary timeout');
console.log('PASS: settings return retains geometry through a slow WebView commit');

// One matching frame is not enough: viewport feedback can be superseded before paint.
Object.assign(window,{innerWidth:330,innerHeight:370});
let finalCommit=false, commitFrame=0;
const originalFrame=globalThis.requestAnimationFrame;
globalThis.requestAnimationFrame=cb=>setTimeout(()=>{
  if(finalCommit) {
    commitFrame++;
    Object.assign(window,commitFrame===2 ? {innerWidth:330,innerHeight:370} : {innerWidth:300,innerHeight:333});
  }
  cb();
},2);
const transient=new Element(300,333); transient.inner=new Element(298,331);
stageNavigation(transient,{copy:new Element(328,368),from:{width:330,height:370},route:'list',previous:'settings'});
let transientSettlement;
await resizeNavigation(transient,async (_,size)=>{if(size.width===300)finalCommit=true;},()=>true,()=>{transientSettlement={width:window.innerWidth,height:window.innerHeight};});
globalThis.requestAnimationFrame=originalFrame;
assert.deepEqual(transientSettlement,{width:300,height:333},'viewport must still match on the confirmation frame before releasing geometry');
console.log('PASS: a transient matching viewport does not settle the return');

// Missing feedback must take error cleanup, never a successful settlement.
Object.assign(window,{innerWidth:330,innerHeight:370});
const missing=new Element(300,333); missing.inner=new Element(298,331);
const missingCopy=new Element(328,368);
stageNavigation(missing,{copy:missingCopy,from:{width:330,height:370},route:'list',previous:'settings'});
let missingSettled=false, missingFinal=false;
const realNow=Date.now;
let virtualTime=realNow();
Date.now=()=>virtualTime;
globalThis.requestAnimationFrame=cb=>setTimeout(()=>{if(missingFinal)virtualTime+=2100;cb();},2);
try {
  await resizeNavigation(missing,async(_,size)=>{if(size.width===300)missingFinal=true;},()=>true,()=>{missingSettled=true;});
  assert.equal(missingSettled,false,'no resize feedback must not be reported as a settled navigation');
  assert.equal(missingCopy.removed,true,'failed viewport confirmation removes the outgoing layer');
  assert.equal(missing.style.width,'','failed confirmation releases temporary styles');
} finally {
  Date.now=realNow;
  globalThis.requestAnimationFrame=originalFrame;
}
console.log('PASS: absent viewport feedback cleans up without a false settlement');
