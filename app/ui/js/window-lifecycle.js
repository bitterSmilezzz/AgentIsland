// Conservatively retain any unsaved form value. Search is disposable UI state.
export class DraftGuard {
  constructor() { this.baselines = new Map(); this.retained = new Set(); }
  retain(controls) { this.retained = new Set(controls); }
  focus(control) {
    if (!this.isFormControl(control)) return;
    if (!this.baselines.has(control)) this.baselines.set(control, this.value(control));
  }
  changed(control) {
    if (!this.isFormControl(control)) return;
    if (!this.baselines.has(control)) {
      this.baselines.set(control, control.type === 'checkbox' ? !!control.defaultChecked : (control.defaultValue ?? ''));
    }
  }
  saved(patch) {
    for (const control of this.baselines.keys()) {
      if (control.dataset?.set && Object.hasOwn(patch, control.dataset.set)) this.baselines.delete(control);
    }
  }
  commit(controls) {
    for (const control of controls) {
      if (this.isFormControl(control)) this.baselines.set(control, this.value(control));
    }
  }
  get dirty() {
    for (const [control, initial] of this.baselines) {
      if (control.isConnected === false && !this.retained.has(control)) { this.baselines.delete(control); continue; }
      if (this.value(control) !== initial) return true;
    }
    return false;
  }
  value(control) { return control.type === 'checkbox' ? !!control.checked : control.value; }
  isFormControl(control) {
    return ['INPUT', 'SELECT', 'TEXTAREA'].includes(control?.tagName) && control.type !== 'search';
  }
}
