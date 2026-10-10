import assert from 'node:assert/strict';
import { retainTaskFocus } from '../app/ui/js/task-focus.js';

function node({ tagName = 'BUTTON', name = '', type = 'button', dataset = {}, form = null, history = false, hidden = false } = {}) {
  return { tagName, name, type, dataset, isConnected: true, disabled: false, focused: false,
    closest(selector) { return selector === 'form' ? form : selector === '.task-history' && history ? {} : selector === '[hidden]' && hidden ? {} : null; },
    focus(options) { this.focused = true; this.focusOptions = options; },
    setSelectionRange(...selection) { this.selection = selection; },
  };
}
const form = { getAttributeNames: () => ['data-task-edit'] };
const old = node({ tagName: 'INPUT', name: 'title', type: 'text', form });
Object.assign(old, { selectionStart: 1, selectionEnd: 3, selectionDirection: 'backward' });
const detail = node({ tagName: 'SECTION', dataset: { taskDetail: '' } });
let controls = [old], disclosures = [{ className: 'task-history', open: true }];
const root = {
  contains: candidate => controls.includes(candidate),
  querySelectorAll: selector => selector === 'details' ? disclosures : controls,
  querySelector: selector => selector.startsWith('[data-task-detail]') ? detail : null,
};
let next;
retainTaskFocus(root, () => {
  old.isConnected = false;
  next = node({ tagName: 'INPUT', name: 'title', type: 'text', form });
  controls = [next]; disclosures = [{ className: 'task-history', open: false }];
}, old);
assert.equal(next.focused, true, 'new source actions can render without losing logical edit focus');
assert.deepEqual(next.selection, [1, 3, 'backward']);
assert.deepEqual(next.focusOptions, { preventScroll: true });
assert.equal(disclosures[0].open, true, 'an expanded run history stays expanded');

const gate = node({ dataset: { taskHandle: 'expired' } }); controls = [gate];
retainTaskFocus(root, () => { gate.isConnected = false; controls = []; }, gate);
assert.equal(detail.focused, true, 'a removed question action returns focus to its detail');

const row = node({ dataset: { taskSelect: 'same-task' } }); controls = [row];
let replacement;
retainTaskFocus(root, () => {
  row.isConnected = false;
  replacement = node({ dataset: { taskSelect: 'same-task' } });
  controls = [node({ dataset: { taskSelect: 'another-task' } }), replacement];
}, row);
assert.equal(replacement.focused, true, 'moving between task groups retains exact task focus');
assert.equal(controls[0].focused, false);

const historyAction = node({ dataset: { taskArtifactRead: 'artifact', artifactRun: 'run' }, history: true });
controls = [historyAction];
retainTaskFocus(root, () => {
  historyAction.isConnected = false;
  controls = [node({ dataset: { taskArtifactRead: 'artifact', artifactRun: 'run' } }),
    node({ dataset: { taskArtifactRead: 'artifact', artifactRun: 'run' }, history: true })];
}, historyAction);
assert.equal(controls[1].focused, true, 'duplicate content actions retain their history context');
assert.equal(controls[0].focused, false);

controls = [next]; next.focused = false;
retainTaskFocus(root, () => {}, next);
assert.equal(next.focused, false, 'retained draft controls are not unnecessarily focused again');
retainTaskFocus(root, () => {}, node());
assert.equal(next.focused, false, 'a background refresh never steals focus from another page');
console.log('PASS: task refresh focus, selection, disclosure, expired actions and background ownership');
