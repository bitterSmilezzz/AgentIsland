// A draft keeps the value it was based on, even after a refresh or route change.
export function createToolBudgetController(read, write, changed = () => {}) {
  const drafts = new Map();
  const state = { report: null, selected: '', busy: false, status: '', readFailed: false };
  const row = () => state.report?.rows.find(item => item.agentId === state.selected);
  const draft = () => drafts.get(state.selected);
  const dirty = item => item && item.value !== String(item.expected);
  const notify = () => changed(controller.state);
  function accept(report) {
    if (!report || !Array.isArray(report.rows)) throw new Error('invalid report');
    state.report = report;
    if (!report.rows.some(item => item.agentId === state.selected)) {
      state.selected = (report.rows.find(item => ['warning', 'exceeded'].includes(item.status))
        ?? report.rows.find(item => item.budget > 0) ?? report.rows.find(item => item.used > 0)
        ?? report.rows[0])?.agentId ?? '';
    }
    for (const item of report.rows) {
      if (!dirty(drafts.get(item.agentId))) drafts.set(item.agentId, { value: String(item.budget), expected: item.budget });
    }
  }
  const controller = {
    get state() {
      const item = draft();
      const value = item?.value ?? '';
      const valid = /^\d+$/.test(value) && Number.isSafeInteger(Number(value)) && Number(value) <= 1e9;
      return { ...state, row: row(), value, dirty: !!dirty(item), valid,
        hasDraft: [...drafts.values()].some(dirty),
        canSave: !!row() && valid && dirty(item) && !state.busy };
    },
    select(id) {
      if (state.busy || !state.report?.rows.some(item => item.agentId === id)) return;
      state.selected = id; state.status = ''; notify();
    },
    edit(value) { if (state.busy || !draft()) return; draft().value = value; state.status = ''; notify(); },
    cancel() {
      if (state.busy || !row()) return;
      drafts.set(state.selected, { value: String(row().budget), expected: row().budget });
      state.status = ''; notify();
    },
    async load() {
      if (state.busy) return;
      state.busy = true; state.status = '正在读取预算'; notify();
      try { accept(await read()); state.readFailed = false; state.status = '已读取当前预算，未保存的输入已保留'; }
      catch { state.readFailed = true; state.status = '预算读取失败，请重试；已有数据和草稿已保留'; }
      finally { state.busy = false; notify(); }
    },
    async save() {
      if (!controller.state.canSave) return;
      const id = state.selected, item = { ...draft() };
      state.busy = true; state.status = '正在保存预算'; notify();
      try {
        const report = await write(id, Number(item.value), item.expected);
        // Validate before discarding a draft. The write may have succeeded if a response was lost.
        if (!report || !Array.isArray(report.rows) || !report.rows.some(row => row.agentId === id)) throw new Error('invalid response');
        drafts.delete(id); accept(report); state.readFailed = false;
        state.status = Number(item.value) === 0 ? '已取消此工具预算' : '预算已保存';
      } catch (error) {
        state.status = typeof error === 'string' ? error : '保存未确认，请刷新核对；草稿已保留';
      } finally { state.busy = false; notify(); }
    },
  };
  return controller;
}
