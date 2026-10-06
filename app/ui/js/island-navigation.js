// One persistent surface: native geometry only reserves/releases a transparent envelope.
// CSS geometry and content share the rendering clock; sampling never restarts navigation.
const transitions = new WeakMap();
const motion = { enter: 420, return: 340, shape: 'cubic-bezier(.32,0,.2,1)', content: 'cubic-bezier(.16,1,.3,1)' };
const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

export function navigationActive(card) { return card != null && transitions.has(card); }
export function captureNavigation(card, previous, route, layoutChange = false) {
  if (!card?.querySelector || previous == null || (previous === route && !layoutChange)) return null;
  const content = card.querySelector('.card-inner');
  if (!content?.cloneNode) return null;
  const rect = card.getBoundingClientRect();
  const running = transitions.get(card);
  const copy = reduced() ? null : content.cloneNode(true);
  if (copy) {
    copy.removeAttribute('id');
    copy.querySelectorAll('[id]').forEach(n => n.removeAttribute('id'));
    const sources = content.querySelectorAll('*');
    copy.querySelectorAll('*').forEach((n, i) => {
      n.scrollTop = sources[i].scrollTop;
      // An interrupted page leaves from what is actually visible, not full opacity.
      if (sources[i].getAnimations().length) {
        const style = getComputedStyle(sources[i]);
        n.style.opacity = style.opacity; n.style.transform = style.transform;
      }
    });
    copy.classList.remove('card-enter');
    copy.setAttribute('aria-hidden', 'true'); copy.inert = true; copy.dataset.pageOutgoing = '';
  }
  if (running) {
    running.revision++; running.animations.forEach(a => a.cancel()); running.copy.remove();
    transitions.delete(card);
    card.style.width = ''; card.style.height = ''; card.style.maxHeight = '';
    delete card.dataset.islandNavigating;
  }
  return copy ? { copy, from: { width: rect.width, height: rect.height }, route, previous, layoutOnly: previous === route, interrupted: !!running } : null;
}

export function stageNavigation(card, captured) {
  if (!captured || !card?.animate) return;
  // Measure the destination before freezing the container at the visible old size.
  card.style.width = ''; card.style.height = ''; card.style.maxHeight = 'none';
  const targetWidth = Math.ceil(card.getBoundingClientRect().width);
  const inner = card.querySelector('.card-inner');
  inner.style.width = `${targetWidth - 2}px`;
  inner.classList.add('island-navigation-content');
  captured.copy.classList.add('island-navigation-content');
  Object.assign(captured.copy.style, { width: `${captured.from.width - 2}px`, height: `${captured.from.height - 2}px` });
  card.appendChild(captured.copy);
  card.dataset.islandNavigating = '';
  Object.assign(card.style, { width: `${captured.from.width}px`, height: `${captured.from.height}px`, maxHeight: 'none' });
  inner.style.visibility = 'hidden';
  transitions.set(card, { ...captured, inner, targetWidth, animations: [], revision: 0, started: false });
}

export function focusNavigation(card, route, previous) {
  const target = route !== 'list' ? card.querySelector('[data-back]')
    : previous === 'settings' ? card.querySelector('[data-island-settings]')
    : previous === 'tokenAnalytics' ? card.querySelector('[data-analytics]')
    : [...card.querySelectorAll('[data-agent]')].find(n => `agentDetail:${n.dataset.agent}` === previous);
  target?.focus({ preventScroll: true });
}

function parts(inner) {
  const page = inner.querySelector('.page');
  const surface = page ?? inner;
  return { header: surface.querySelector('.page-header, .header'), body: [...surface.children].filter(n => !n.matches('.page-header, .header, .divider')) };
}
function playContent(entry) {
  const returning = entry.route === 'list';
  const old = parts(entry.copy), next = parts(entry.inner);
  const play = (node, frames, options) => {
    if (node?.animate) entry.animations.push(node.animate(frames, { fill: 'both', ...options }));
  };
  // Geometry owns movement. Extra text translations oppose the shrinking
  // settings surface and look like a bounce even when the border is monotonic.
  play(old.header, [{ opacity: Number(old.header ? getComputedStyle(old.header).opacity : 1) }, { opacity: 0 }], { duration: returning ? 80 : 100, easing: 'ease-out' });
  old.body.forEach(n => play(n, [{ opacity: Number(getComputedStyle(n).opacity) }, { opacity: 0 }], { duration: returning ? 100 : 125, easing: 'ease-out' }));
  play(next.header, [{ opacity: 0 }, { opacity: 1 }], { duration: 230, delay: entry.interrupted ? 0 : returning ? 80 : 105, easing: motion.content });
  next.body.forEach((n, index) => play(n, [{ opacity: 0 }, { opacity: 1 }], { duration: returning ? 245 : 300, delay: (entry.interrupted ? 20 : returning ? 85 : 115) + Math.min(index, 3) * 12, easing: motion.content }));
}

// IPC completion acknowledges native work, not WebKit's viewport commit.
// Keep frozen geometry until both clocks agree before sampling may lay out again.
async function awaitViewport(target, active) {
  const deadline = Date.now() + 2000;
  const matches = () => Math.abs(window.innerWidth - target.width) <= 1 && Math.abs(window.innerHeight - target.height) <= 1;
  do {
    await new Promise(requestAnimationFrame);
    if (!active()) return false;
    if (matches()) {
      await new Promise(requestAnimationFrame);
      if (!active()) return false;
      if (matches()) return true;
    }
  } while (Date.now() < deadline);
  // A timeout is failure, never proof of a committed viewport. The caller
  // releases temporary layers through its error cleanup without declaring settlement.
  throw new Error('Island viewport did not commit the requested geometry');
}

export function resizeNavigation(card, invoke, valid, settled) {
  const entry = transitions.get(card);
  if (!entry) return null;
  const report = entry.inner.querySelector('[data-report-root]');
  const naturalHeight = entry.inner.getBoundingClientRect().height + 2;
  // A loading placeholder must not collapse the surface before the data arrives.
  const height = report && !report.dataset.reportReady ? Math.max(naturalHeight, entry.from.height) : naturalHeight;
  const target = { width: entry.targetWidth, height: Math.ceil(Math.min(height, 520)) };
  const key = `${target.width}:${target.height}`;
  if (entry.key === key && entry.promise) return entry.promise;
  entry.key = key;
  const revision = ++entry.revision;
  const current = card.getBoundingClientRect();
  const previousGeometry = entry.geometry;
  entry.animations = entry.animations.filter(a => a !== previousGeometry);
  previousGeometry?.cancel();
  Object.assign(card.style, { width: `${current.width}px`, height: `${current.height}px` });
  const duration = entry.layoutOnly ? 220 : entry.route === 'list' ? motion.return : motion.enter;
  const active = () => transitions.get(card) === entry && entry.revision === revision && valid();
  entry.promise = (async () => {
    // Dock-aligned root keeps the visible old surface anchored while the transparent
    // window grows. Never reposition/resize the native window on every visual frame.
    const envelope = { width: Math.ceil(Math.max(window.innerWidth, current.width, target.width)), height: Math.ceil(Math.max(window.innerHeight, current.height, target.height)) };
    await invoke('place_island', envelope);
    if (!await awaitViewport(envelope, active)) return;
    if (!entry.started) {
      entry.started = true;
      entry.inner.style.visibility = '';
      if (entry.layoutOnly) entry.copy.remove();
      else {
        focusNavigation(card, entry.route, entry.previous);
        playContent(entry);
      }
    }
    const shape = card.animate([{ width: `${current.width}px`, height: `${current.height}px` }, { width: `${target.width}px`, height: `${target.height}px` }], { duration, easing: motion.shape, fill: 'both' });
    entry.geometry = shape;
    entry.animations.push(shape);
    await shape.finished.catch(() => {});
    if (!active()) return;
    // All layers finish before the native envelope contracts. No clipped last frame.
    await Promise.all(entry.animations.map(a => a.finished.catch(() => {})));
    if (!active()) return;
    Object.assign(card.style, { width: `${target.width}px`, height: `${target.height}px` });
    await invoke('place_island', target);
    if (!await awaitViewport(target, active)) return;
    entry.animations.forEach(a => a.cancel());
    entry.copy.remove();
    entry.inner.classList.remove('island-navigation-content');
    entry.inner.style.width = '';
    card.style.width = ''; card.style.height = ''; card.style.maxHeight = '';
    delete card.dataset.islandNavigating;
    transitions.delete(card);
    settled();
  })().catch(() => {
    if (transitions.get(card) !== entry || entry.revision !== revision) return;
    entry.animations.forEach(a => a.cancel()); entry.copy.remove();
    entry.inner.classList.remove('island-navigation-content'); entry.inner.style.width = ''; entry.inner.style.visibility = '';
    card.style.width = ''; card.style.height = ''; card.style.maxHeight = '';
    delete card.dataset.islandNavigating; transitions.delete(card);
  });
  return entry.promise;
}
