// Snapshots may replace task actions while a clean control has focus. Keep the
// logical control, selection and disclosure state without retaining stale data.
const identity = node => JSON.stringify([
  node.tagName, node.name ?? '', node.type ?? '',
  Boolean(node.closest('.task-history')),
  node.closest('form')?.getAttributeNames().filter(name => name.startsWith('data-task-')).sort() ?? [],
  Object.entries(node.dataset ?? {}).filter(([name]) => /^(task|artifact)/.test(name)).sort(),
]);

export function retainTaskFocus(root, render, active = document.activeElement) {
  const owned = root.contains(active), key = owned ? identity(active) : null;
  const selection = owned && typeof active.selectionStart === 'number'
    ? [active.selectionStart, active.selectionEnd, active.selectionDirection] : null;
  const disclosures = [...root.querySelectorAll('details')].map(node => [node.className, node.open]);
  render();
  for (const node of root.querySelectorAll('details')) {
    const saved = disclosures.find(([name]) => name === node.className);
    if (saved) node.open = saved[1];
  }
  if (!owned || active.isConnected && root.contains(active)) return;
  const target = [...root.querySelectorAll('input, select, button, summary, [tabindex]')]
    .find(node => !node.disabled && !node.closest('[hidden]') && identity(node) === key)
    ?? root.querySelector('[data-task-detail]:not([hidden])')
    ?? root.querySelector('[data-task-filter]');
  target?.focus({ preventScroll: true });
  if (selection && target && identity(target) === key) target.setSelectionRange?.(...selection);
}
