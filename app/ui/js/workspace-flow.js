// One explicit workspace workflow per WebView. Receipts describe past operations,
// never current client state; no configuration contents or window titles are retained.
const routes = { project: 'tasks', profile: 'provider', layout: 'windows', tool: 'agents' };
const writable = new Set(['profile', 'layout']);
const backupName = value => typeof value === 'string' && /^config-\d+\.toml$/.test(value) ? value : null;
export class WorkspaceFlow {
  constructor() { this.session = null; this.intent = null; this.pending = null; this.onChange = () => {}; }
  operationContext(kind,target,recovery=false) {
    const s=this.session,i=this.intent;
    if(kind!=='profile'||!s||s.stale||!i||i.kind!==kind||i.recovery!==recovery||(recovery?i.receipt?.recovery_id!==target:i.target!==target))return null;
    if(recovery&&!i.receipt?.operation_id)return null;
    return {id:s.id,expected_revision:s.revision,profile_id:i.target,recovery_operation_id:recovery?i.receipt.operation_id:null};
  }
  layoutContext(target,rulesRevision,recovery=false){
    const s=this.session,i=this.intent;
    if(!s||s.stale||!i||i.kind!=='layout'||i.recovery!==recovery||(recovery?i.receipt?.recovery_id!==target:i.target!==target))return null;
    return {id:s.id,expected_revision:s.revision,layout_id:i.target,expected_rules_revision:rulesRevision};
  }
  prepareLayoutUndo(operationId) {
    const s=this.session,i=this.intent,receipt=s?.receipts.get('layout');
    if(!s||s.stale||i?.kind!=='layout'||i.recovery||receipt?.recovery_id!==operationId||!receipt.recoverable)return;
    const step=s.steps.find(step=>step.kind==='layout'&&step.target_id===i.target&&step.available);
    if(step)this.activate(step,true);
  }
  open(preview, records=[],windowRecords=null) {
    if (this.pending) throw new Error('当前分项仍在执行，请完成后再打开组合。');
    const same = this.session?.id === preview.workspace.id && this.session?.revision === preview.revision;
    const receipts = same ? this.session.receipts : new Map();
    if(!same){
      const target=preview.steps.find(s=>s.kind==='profile')?.target_id;
      const history=records.filter(r=>r.workspace_id===preview.workspace.id&&r.profile_id===target);
      const latest=history.at(-1),recover=[...history].reverse().find(r=>['applied','restored'].includes(r.phase)&&backupName(r.backup_name));
      if(latest)receipts.set('profile',{state:'historical',detail:latest.phase==='pending'?'上次操作未记录结果，请先在配置页核对；不会自动重试。':latest.phase==='failed'?'上次操作未完成，请重新核对。':'历史配置操作已记录；当前生效状态需在工具内核对。',recoverable:!!recover,recovery_id:recover?.backup_name,backup:recover?.backup_name,operation_id:recover?.operation_id});
    }
    if(windowRecords!==null&&(!same||receipts.get('layout')?.historical)){
      const layout=preview.steps.find(s=>s.kind==='layout')?.target_id;
      const windows=windowRecords.filter(r=>r.workspace?.id===preview.workspace.id&&r.workspace?.layout_id===layout).sort((a,b)=>a.created_ms-b.created_ms);
      const last=windows.at(-1);
      if(last)receipts.set('layout',{state:'historical',detail:last.phase==='pending'?'上次排列结果待核对；选择当前窗口，预览历史原位置再恢复。':last.phase==='failed'?'上次排列未执行；历史原位置可供核对。':'历史逐窗结果已记录；当前窗口位置需重新核对。',recoverable:!!last.slots?.length,recovery_id:last.id,historical:true});

      if(!last)receipts.delete('layout');
    }
    this.session = { id: preview.workspace.id, revision: preview.revision, name: preview.workspace.draft.name,
      steps: preview.steps.map(s => ({ ...s })), receipts };
    this.intent = null; this.onChange();
  }
  reconcile(list) {
    if (!this.session) return;
    if (list.revision !== this.session.revision || !list.items.some(w => w.id === this.session.id)) {
      this.session.stale = true; this.intent = null; this.onChange();
    }
  }
  activate(step, recovery = false) {
    if (this.pending) throw new Error('当前分项仍在执行。');
    const s = this.session;
    if (!s || s.stale || !step.available || !s.steps.some(x => x.kind === step.kind && x.target_id === step.target_id && x.available))
      throw new Error('组合或引用已变化，请重新打开核对。');
    const receipt = s.receipts.get(step.kind);
    if (recovery && (!receipt || !receipt.recoverable)) throw new Error('当前没有可恢复的操作记录。');
    this.intent = { kind: step.kind, target: step.target_id, recovery, receipt };
    this.onChange();
  }
  route(page) {
    if (this.intent && routes[this.intent.kind] !== page) { this.intent = null; this.onChange(); }
  }
  receipt(kind) { return this.session?.receipts.get(kind); }
  // Freshly validate the composition before invoking the original module command.
  // The original command still owns config/window revisions and the actual write.
  async execute(kind, target, verify, command, recovery = false) {
    if(!writable.has(kind))return command();
    const intent = this.intent, session = this.session;
    const matches = intent && session && !session.stale && intent.kind === kind && intent.recovery === recovery &&
      (recovery ? intent.receipt?.recovery_id === target : intent.target === target);
    if (!matches) { if (intent && !this.pending) { this.intent=null; this.onChange(); } return command(); }
    if (this.pending) throw new Error('工作空间分项仍在执行。');
    const lease = { session, kind, intent }; this.pending = lease; this.onChange();
    let dispatched=false;
    try {
      const checked = await verify({ id: session.id, expectedRevision: session.revision });
      if (checked.workspace.id !== session.id || checked.revision !== session.revision ||
          !checked.steps.some(s => s.kind === kind && s.target_id === intent.target && s.available)) {
        session.stale = true;
        throw new Error('工作空间或目标已变化，请重新打开核对。');
      }
      dispatched=true;
      const result = await command();
      const receipt = this.summarize(kind, result, recovery);
      if (recovery && receipt.state === 'restored' && receipt.recovery_id) {
        // A config restore creates a fresh recovery backup too; retain it for inspection.
        receipt.previous_backup = intent.receipt.recovery_id;
      }
      session.receipts.set(kind, receipt);
      return result;
    } catch (error) {
      if(!dispatched)session.stale=true;
      const old = session.receipts.get(kind);
      // A failed attempt does not erase a previous successful operation's recovery.
      session.receipts.set(kind, { ...old, state: 'failed', detail: recovery ? '恢复未完成，请在原模块核对结果与备份。' : '本次未完成，请核对后重新预览。' });
      throw error;
    } finally {
      this.pending = null; this.onChange();
    }
  }
  summarize(kind, result, recovery) {
    if (kind === 'profile') {
      const backup = backupName(result?.backup_name);
      if (!backup) return { state: 'uncertain', detail: '未收到完整配置收据，请在配置页核对。' };
      return { state: result.record_warning ? 'warning' : recovery ? 'restored' : 'applied',
        detail: result.record_warning ? '配置已发布，应用记录未更新。' : recovery ? '配置已还原；生效情况以新会话为准。' : '配置已写入；生效情况以新会话为准。',
        recoverable: true, recovery_id: backup, backup, operation_id: result.operation_id };
    }
    const rows = result?.windows;
    if (!Array.isArray(rows) || !rows.length || typeof result?.operation_id !== 'string')
      return { state: 'uncertain', detail: '未收到逐窗结果，请在窗口页核对。' };
    const good = rows.filter(r => r.status === (recovery ? 'restored' : 'applied') || (recovery&&r.status==='applied')).length;
    const remaining = rows.length - good;
    return { state: result.record_warning?'warning':remaining ? good ? 'warning' : 'failed' : recovery ? 'restored' : 'applied',
      detail: `${good} 个窗口${recovery ? '已恢复' : '已排列'}${remaining ? `，${remaining} 个需处理` : ''}。${result.record_warning?'历史记录未结算，请核对原模块；勿重复排列。':''}`,
      recoverable: result.undo_available === true, recovery_id: result.operation_id };
  }
}
export const workspaceFlow = new WorkspaceFlow();
export const workspaceRoute = kind => routes[kind];
export const workspaceWritable = kind => writable.has(kind);
export const workspaceReceiptLabel = state => ({ applied: '本次已应用', warning: '部分完成', failed: '需处理', restored: '本次已恢复', uncertain: '待核对', historical:'历史记录' }[state] ?? '未应用');
