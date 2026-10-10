import { isWorkbench } from './shell.js';

// Keep the outgoing page visible while the destination arrives.
const outgoingPages = new WeakMap();
const pageTransitions = new WeakMap();
const opacityOf = node => {
  const value = Number(getComputedStyle(node).opacity);
  return Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 1;
};
export function rememberPage(root, key, selector) {
  const previous = root.dataset.motionRoute;
  const node = root.querySelector(selector);
  if (previous == null || previous === key || !node?.cloneNode) return;
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  let source = node, sourceRoute = previous;
  // During the exit phase the destination is still hidden. Capture the actual
  // visible source, or resume it directly when the user reverses navigation.
  if (isWorkbench()) {
    const visible = [...root.querySelectorAll('[data-page-outgoing]')].find(copy =>
      copy.dataset.pageCovered != null && copy.dataset.pageSurface === selector);
    if (visible) { source = visible; sourceRoute = visible.dataset.pageSource; }
  }
  const opacity = opacityOf(source);
  let snapshot;
  if (!reduced && isWorkbench() && sourceRoute === key) snapshot = { resumeOpacity: opacity };
  else if (!reduced && (!isWorkbench() || opacity > .01)) {
    const rect = source.getBoundingClientRect();
    const parentRect = node.parentElement.getBoundingClientRect();
    const copy = source.cloneNode(true);
    const positions = [source, ...source.querySelectorAll('*')].map(n => [n.scrollLeft, n.scrollTop]);
    copy.querySelectorAll('[id]').forEach(n => n.removeAttribute('id'));
    copy.removeAttribute('id');
    copy.classList.remove('card-enter');
    copy.setAttribute('aria-hidden', 'true');
    copy.inert = true;
    copy.dataset.pageOutgoing = '';
    copy.dataset.pageSource = sourceRoute;
    copy.dataset.pageSurface = selector;
    snapshot = { copy, width: rect.width, height: rect.height, top: rect.top - parentRect.top, opacity,
      restoreScroll() {
        [copy, ...copy.querySelectorAll('*')].forEach((n, i) => {
          [n.scrollLeft, n.scrollTop] = positions[i];
        });
      } };
  }
  pageTransitions.get(root)?.forEach(animation => animation.cancel());
  root.querySelectorAll('[data-page-outgoing]').forEach(n => n.remove());
  root.querySelectorAll('[data-page-cover]').forEach(n => n.remove());
  root.querySelectorAll('.page-motion-host').forEach(host => host.classList.remove('page-motion-host'));
  pageTransitions.delete(root);
  outgoingPages.delete(root);
  if (snapshot) outgoingPages.set(root, snapshot);
}
export function pageMotion(root, key, selector) {
  const previous = root.dataset.motionRoute;
  root.dataset.motionRoute = key;
  const changed = previous != null && previous !== key;
  const node = root.querySelector(selector);
  const outgoing = outgoingPages.get(root);
  outgoingPages.delete(root);
  if (changed && node?.animate && !matchMedia('(prefers-reduced-motion: reduce)').matches) {
    const returning = key === 'list' || key === 'overview';
    const direction = returning ? -1 : 1;
    const duration = returning ? 260 : 320;
    const animations = [];
    pageTransitions.set(root, animations);
    const stationary = isWorkbench();
    const exitDuration = stationary ? 70 : duration * .7;
    if (outgoing?.copy) {
      const host = node.parentElement;
      host.classList.add('page-motion-host');
      const { copy, width, height, top, opacity } = outgoing;
      // Keep a solid plate until handoff. Fade only its content: fading the
      // entire plate would reveal both text layers; fading to zero flashes blank.
      const cover = stationary ? document.createElement('div') : copy;
      if (stationary) {
        cover.dataset.pageCover = '';
        cover.setAttribute('aria-hidden', 'true'); cover.inert = true;
        cover.style.background = 'var(--surface)';
        copy.dataset.pageCovered = '';
        cover.appendChild(copy);
        Object.assign(copy.style, { position: 'absolute', top: '0', left: '0', width: `${width}px`, height: `${height}px`, margin: '0', overflow: 'hidden' });
      }
      Object.assign(cover.style, { position: 'absolute', top: `${top}px`, left: '0', width: `${width}px`, height: `${height}px`, margin: '0', zIndex: '2', pointerEvents: 'none', overflow: 'hidden' });
      if (stationary) {
        const target = node.getBoundingClientRect(), parent = host.getBoundingClientRect();
        Object.assign(cover.style, { top: `${target.top-parent.top}px`, left: `${target.left-parent.left}px`,
          width: `${target.width}px`, height: `${Math.max(height,target.height)}px` });
      }
      host.appendChild(cover);
      outgoing.restoreScroll();
      const leaving = copy.animate([{ opacity, transform: 'translateX(0)' }, { opacity: stationary ? .4 : 0, transform: stationary ? 'none' : `translateX(${-direction * 10}px)` }], { duration: exitDuration, easing: 'cubic-bezier(.4,0,.2,1)', fill: 'forwards' });
      animations.push(leaving);
      leaving.finished.catch(() => {}).finally(() => { cover.remove(); if (pageTransitions.get(root) === animations) host.classList.remove('page-motion-host'); });
    }
    animations.push(node.animate([
      { opacity: outgoing?.resumeOpacity ?? (stationary ? .4 : 0), transform: stationary ? 'none' : `translateX(${direction * 14}px)` },
      { opacity: 1, transform: 'translateX(0)' },
    ], { duration: stationary ? 180 : duration, delay: stationary && outgoing?.copy ? exitDuration : 0, fill: 'backwards', easing: 'cubic-bezier(.22,1,.36,1)' }));
    Promise.all(animations.map(a => a.finished.catch(() => {}))).then(() => {
      if (pageTransitions.get(root) === animations) pageTransitions.delete(root);
    });
  }
  return changed;
}
