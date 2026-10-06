import assert from 'node:assert/strict';
import { stageNavigation, resizeNavigation } from '../app/ui/js/island-navigation.js';
const frames=[];
class Node {
 constructor(width=300,height=280,header=false){Object.assign(this,{width,height,header,style:{},dataset:{},children:[],classList:{add(){},remove(){}}});}
 getBoundingClientRect(){return {width:this.width,height:this.height};}
 querySelector(s){if(s==='.card-inner')return this.inner;if(s==='.page')return null;if(s==='.page-header, .header')return this.children.find(n=>n.header);return null;}
 querySelectorAll(){return [];}
 matches(){return this.header;}
 appendChild(){} remove(){}
 animate(f){frames.push({node:this,frames:f});return {finished:Promise.resolve(),cancel(){}};}
}
globalThis.window={innerWidth:330,innerHeight:412};
globalThis.requestAnimationFrame=f=>setTimeout(f,1);
globalThis.getComputedStyle=()=>({opacity:'1',transform:'none'});
const card=new Node();card.inner=new Node(298,278);card.inner.children=[new Node(0,0,true),new Node()];
const old=new Node(328,410);old.children=[new Node(0,0,true),new Node()];
stageNavigation(card,{copy:old,from:{width:330,height:412},route:'list',previous:'settings'});
await resizeNavigation(card,async(_,s)=>Object.assign(window,{innerWidth:s.width,innerHeight:s.height}),()=>true,()=>{});
assert(frames.filter(f=>f.node!==card).every(f=>f.frames.every(k=>!k.transform||k.transform==='none')),'settings return must have one motion source: the surface; independent text translations create opposing movement');
console.log('PASS: settings return changes surface geometry while content only fades');
