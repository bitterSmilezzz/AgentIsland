import assert from 'node:assert/strict';
import { assessMotionTrace } from '../app/ui/js/motion-probe.js';
import { invoke, setMotionProbeReportDelay } from '../app/ui/js/tauri.js';

let enabled=false;
const nativeInvoke=async command=>{
  if(command==='motion_probe_frame' && !enabled) throw new Error('disabled');
  return {command};
};
globalThis.window={__TAURI__:Object.freeze({core:Object.freeze({invoke:nativeInvoke})})};
await assert.rejects(setMotionProbeReportDelay(650), /disabled/);
enabled=true;
await assert.rejects(setMotionProbeReportDelay(1), /Invalid/);
await setMotionProbeReportDelay(650);
const started=performance.now();
assert.deepEqual(await invoke('get_report'),{command:'get_report'});
assert.ok(performance.now()-started>=630,'late report is delayed without replacing frozen Tauri API');
await setMotionProbeReportDelay(0);
assert.equal(window.__TAURI__.core.invoke,nativeInvoke);
delete globalThis.window;

function fixture(edge='top') {
  const frames=Array.from({length:30},(_,index)=>{
    const progress=Math.min(index/20,1),width=300+30*progress,height=333+67*progress;
    const x=edge==='left'?0:edge==='right'?400-width:(400-width)/2;
    const y=edge==='top'?0:edge==='bottom'?520-height:(520-height)/2;
    return {t:index*16,native:{x:100,y:50,width:400,height:520,visible:true},
      viewport:{width:400,height:520},card:{x,y,width,height},navigating:index<21};
  });
  return {edge,frames,reduced:false};
}
for(const edge of ['top','bottom','left','right']) assert.equal(assessMotionTrace(fixture(edge)).passed,true,edge);
function reject(name,mutate) {
  const trace=fixture();mutate(trace);
  assert.ok(assessMotionTrace(trace).failures.includes(name),name);
}
reject('anchor-drift',trace=>{trace.frames[15].native.x+=5;});
reject('clipped-surface',trace=>{trace.frames[15].card.y=-5;});
reject('hidden-native-window',trace=>{trace.frames[15].native.visible=false;});
reject('invalid-geometry',trace=>{trace.frames[15].native.width=NaN;});
reject('insufficient-frames',trace=>{trace.frames=trace.frames.slice(0,2);});
reject('non-monotonic-clock',trace=>{trace.frames[15].t=0;});
reject('unsettled-final-frame',trace=>{trace.frames.at(-1).navigating=true;});
reject('reversal-width',trace=>{
  trace.frames[15].card.width-=15;
  trace.frames[15].card.x=(400-trace.frames[15].card.width)/2;
});
const jump=fixture();jump.frames.forEach((frame,index)=>{
  const source=fixture().frames[index<15?0:29];frame.card={...source.card};
});
assert.ok(assessMotionTrace(jump).failures.includes('missing-intermediate-width'));
jump.reduced=true;
assert.equal(assessMotionTrace(jump).passed,true,'reduced motion can jump without violating anchoring');
const commit=fixture();commit.frames.slice(0,5).forEach(frame=>{frame.native.width+=100;});
assert.equal(assessMotionTrace(commit).passed,true,'transient WebKit/native commit mismatch is not geometric drift');
commit.frames.slice(-3).forEach(frame=>{frame.native.width+=100;});
assert.ok(assessMotionTrace(commit).failures.includes('unsettled-final-frame'));
const rapid=fixture();rapid.interrupted=true;
rapid.frames[15].card.width-=15;rapid.frames[15].card.x=(400-rapid.frames[15].card.width)/2;
assert.equal(assessMotionTrace(rapid).passed,true,'explicit reverse navigation permits a direction change');
console.log('PASS: native motion traces reject anchor drift, clipping, reversal, hidden/invalid/insufficient frames and false settlement');
