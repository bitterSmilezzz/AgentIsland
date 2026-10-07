// Loaded only by the explicit visible native regression mode. No user content is recorded.
import { setMotionProbeReportDelay } from './tauri.js';
const tolerance = 2;
const rectKeys = ['x', 'y', 'width', 'height'];
const finiteRect = value => value && rectKeys.every(key => Number.isFinite(value[key])) && value.width > 0 && value.height > 0;
const spread = values => Math.max(...values) - Math.min(...values);
const percentile = (values, ratio) => [...values].sort((a,b) => a-b)[Math.ceil(values.length*ratio)-1] ?? null;

export function assessMotionTrace(trace) {
  const failures = [];
  const frames = trace.frames ?? [];
  if (!['top','bottom','left','right'].includes(trace.edge)) failures.push('invalid-edge');
  if (frames.length < 12) failures.push('insufficient-frames');
  const valid = frames.filter(frame => finiteRect(frame.card) && finiteRect(frame.native)
    && Number.isFinite(frame.t) && Number.isFinite(frame.viewport?.width) && Number.isFinite(frame.viewport?.height));
  if (valid.length !== frames.length) failures.push('invalid-geometry');
  if (valid.some((frame,index) => index && frame.t <= valid[index-1].t)) failures.push('non-monotonic-clock');
  if (valid.some(frame => !frame.native.visible)) failures.push('hidden-native-window');
  // Native submission and WebKit viewport commits have separate clocks. Only
  // compare their coordinate systems when the envelope dimensions agree.
  const matched = valid.filter(frame => Math.abs(frame.native.width-frame.viewport.width) <= tolerance
    && Math.abs(frame.native.height-frame.viewport.height) <= tolerance);
  if (matched.length < 12) failures.push('insufficient-matched-frames');
  if (matched.some(({card,viewport}) => card.x < -tolerance || card.y < -tolerance
    || card.x+card.width > viewport.width+tolerance || card.y+card.height > viewport.height+tolerance)) failures.push('clipped-surface');
  const anchors = matched.map(({card,native}) => {
    const left=native.x+card.x, top=native.y+card.y;
    return trace.edge === 'top' ? [left+card.width/2,top]
      : trace.edge === 'bottom' ? [left+card.width/2,top+card.height]
      : trace.edge === 'left' ? [left,top+card.height/2]
      : [left+card.width,top+card.height/2];
  });
  if (anchors.length && [0,1].some(axis => spread(anchors.map(anchor => anchor[axis])) > tolerance)) failures.push('anchor-drift');
  const tail=valid.slice(-3);
  if (tail.length && (tail.some(frame => frame.navigating)
    || ['width','height'].some(key => spread(tail.map(frame => frame.card[key])) > 1)
    || !matched.includes(tail.at(-1)))) failures.push('unsettled-final-frame');
  if (!trace.interrupted && !trace.lateContent && valid.length) {
    for (const key of ['width','height']) {
      const delta=valid.at(-1).card[key]-valid[0].card[key];
      if (Math.abs(delta) <= tolerance) continue;
      const direction=Math.sign(delta);
      if (valid.some((frame,index) => index && (frame.card[key]-valid[index-1].card[key])*direction < -tolerance)) failures.push(`reversal-${key}`);
      if (!trace.reduced && !valid.some(frame => Math.abs(frame.card[key]-valid[0].card[key]) > tolerance
        && Math.abs(frame.card[key]-valid.at(-1).card[key]) > tolerance)) failures.push(`missing-intermediate-${key}`);
    }
  }
  return { passed: failures.length===0, failures:[...new Set(failures)], frames:frames.length,
    matchedFrames:matched.length, frameIntervalP95:percentile(valid.slice(1).map((frame,index)=>frame.t-valid[index].t),.95) };
}

export async function runMotionProbe() {
  const invoke=window.__TAURI__.core.invoke.bind(window.__TAURI__.core);
  const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
  const nextFrame=()=>new Promise(requestAnimationFrame);
  const card=()=>document.querySelector('.card');
  const readRect=node=>{const rect=node.getBoundingClientRect();return Object.fromEntries(rectKeys.map(key=>[key,rect[key]]));};
  const deadline=performance.now()+15000;
  const traces=[];
  let edge;
  try {
    while (!card() || !document.querySelector('[data-agent]') || card().style.visibility==='hidden') {
      if(performance.now()>deadline) throw new Error('synthetic-island-not-ready');
      await wait(50);
    }
    await wait(600);
    edge=['top','bottom','left','right'].find(value=>document.documentElement.classList.contains(`edge-${value}`));
    async function capture(label,selector,options={}) {
      const target=document.querySelector(selector);
      if(!target) throw new Error('missing-probe-control');
      const trace={label,edge,reduced:matchMedia('(prefers-reduced-motion: reduce)').matches,
        interrupted:!!options.interrupted,lateContent:!!options.lateContent,frames:[]};
      let clicked=false, reversed=false;
      const start=performance.now(), duration=options.lateContent?2800:1800;
      do {
        const native=await invoke('motion_probe_frame');
        await nextFrame();
        trace.frames.push({t:performance.now()-start,native,card:readRect(card()),
          viewport:{width:innerWidth,height:innerHeight},navigating:card().hasAttribute('data-island-navigating')});
        if(!clicked){target.click();clicked=true;}
        if(options.interrupted && !reversed && performance.now()-start>100) {
          const back=document.querySelector('[data-back]');
          if(!back) throw new Error('missing-reverse-control');
          back.click();reversed=true;
        }
      } while(performance.now()-start<duration);
      trace.assessment=assessMotionTrace(trace);traces.push(trace);
      if(!trace.assessment.passed) throw new Error('motion-invariant-failed');
    }
    await capture('settings-enter','[data-island-settings]');
    await capture('settings-return','[data-back]');
    await capture('usage-enter','[data-analytics]');
    await capture('usage-return','[data-back]');
    await capture('agent-enter','[data-agent]');
    await capture('agent-return','[data-back]');
    await capture('rapid-settings-return','[data-island-settings]',{interrupted:true});
    await setMotionProbeReportDelay(650);
    await capture('late-usage','[data-analytics]',{lateContent:true});
    await setMotionProbeReportDelay(0);
    await capture('late-usage-return','[data-back]');
    await invoke('log_from_ui',{message:'MOTION_RESULT '+JSON.stringify({schema:1,edge,passed:true,traces})});
    await invoke('motion_probe_finish',{passed:true});
  } catch (error) {
    const known=['synthetic-island-not-ready','missing-probe-control','missing-reverse-control','motion-invariant-failed'];
    const failure=known.includes(error?.message)?error.message:'probe-runtime-failed';
    await invoke('log_from_ui',{message:'MOTION_RESULT '+JSON.stringify({schema:1,edge,passed:false,failure,traces})});
    await invoke('motion_probe_finish',{passed:false});
  } finally {
    await setMotionProbeReportDelay(0).catch(()=>{});
  }
}
