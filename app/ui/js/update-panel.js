import { invoke, listen } from './tauri.js';
import { createUpdateController, updateStatusText, formatBytes, errorText } from './update-state.js';

const BUSY = new Set(['restoring', 'checking', 'downloading', 'skipping', 'restarting']);
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

/** 「检查更新」分区。**默认不检查**——挂载时先恢复进程级待重启状态；无待更新时显示「尚未检查更新」。
 *  容器带 `data-update` 而不是 `data-update-root`：项目里已有一个 `data-draft-ignore`
 *  之类的约定，这里用独立前缀避免与其它模块的选择器冲突。 */
export function updatePanelHtml() {
  return `<section class="settings-section update-section" data-update-panel>
    <h2>更新</h2>
    <p class="update-note">仅在你点击时检查与安装更新；跳过只对当前版本生效，新版本仍会提示。安装完成后需你再次确认才会重启。</p>
    <p class="update-current" data-update-current></p>
    <div class="update-actions">
      <button type="button" class="mini-btn" data-update-check>检查更新</button>
      <button type="button" class="mini-btn" data-update-install hidden>下载并安装</button>
      <button type="button" class="mini-btn" data-update-skip hidden>跳过此版本</button>
      <button type="button" class="mini-btn" data-update-restore-skip hidden>恢复更新提示</button>
      <button type="button" class="mini-btn" data-update-restart hidden>立即重启</button>
      <button type="button" class="mini-btn" data-update-dismiss hidden>稍后再说</button>
    </div>
    <p class="update-status" data-update-status role="status" aria-live="polite"></p>
    <pre class="update-notes" data-update-notes hidden></pre>
  </section>`;
}

/**
 * 绑定更新分区。返回 controller，便于测试与外部读取。
 *
 * 进度事件在**每次绑定**时注册一次，并在解绑时取消——否则重复进入设置页会叠加
 * 监听器，导致一次下载收到多条进度。
 */
export function bindUpdatePanel(root = document) {
  // 调用方可能传 **document**（默认值），也可能传设置页根**元素**（`bindSettings`
  // 传的就是元素）。两种都得能用：元素本身没有 `querySelector`，直接调用会在
  // 挂载时抛错，把整个设置页打不开——这正是既有回归测试替我们抓到的。
  const scope = typeof root?.querySelector === 'function' ? root : root?.ownerDocument ?? root;
  const panel = scope?.querySelector?.('[data-update-panel]') ?? null;
  if (!panel) return null;
  if (panel.dataset.bound === 'true') return panel.__updateController ?? null;
  panel.dataset.bound = 'true';
  const find = key => panel.querySelector(`[data-update-${key}]`);

  let controller = null;
  // 渲染与状态机分开：`controller` 在回调被调用之前还没建好，所以按钮的可用性
  // 一律从 `state` 直接推导，不反向读 controller——这既避开 TDZ，也让渲染函数
  // 可以在绑定完成后立刻调用一次来铺初始状态。
  const render = state => {
    find('current').textContent = state.currentVersion ? `当前版本 v${esc(state.currentVersion)}` : '';
    find('status').textContent = updateStatusText(state);
    // 按钮的可见性就是「这条路径此刻允许吗」的状态机投影，不要另写一套条件。
    find('check').hidden = !['idle', 'failed', 'skipped'].includes(state.phase);
    find('check').disabled = BUSY.has(state.phase);
    find('check').textContent = state.phase === 'failed' ? '重试' : '检查更新';
    const install = find('install');
    install.hidden = state.phase !== 'available';
    install.disabled = BUSY.has(state.phase);
    install.textContent = state.phase === 'downloading'
      ? (state.total ? `${Math.min(100, Math.floor((state.downloaded ?? 0) / state.total * 100))}%` : '下载中…')
      : '下载并安装';
    const skip = find('skip');
    skip.hidden = state.phase !== 'available';
    skip.disabled = BUSY.has(state.phase);
    const restoreSkip = find('restore-skip');
    restoreSkip.hidden = !state.skippedVersion || BUSY.has(state.phase);
    restoreSkip.disabled = BUSY.has(state.phase);
    const restart = find('restart');
    restart.hidden = state.phase !== 'ready';
    restart.disabled = BUSY.has(state.phase);
    const dismiss = find('dismiss');
    dismiss.hidden = !['available', 'failed', 'rejected', 'uptodate', 'skipped'].includes(state.phase);
    dismiss.textContent = state.phase === 'rejected' ? '关闭' : '稍后再说';
    const notes = find('notes');
    const text = state.notes || '';
    notes.hidden = !text;
    notes.textContent = text;
    panel.setAttribute('aria-busy', String(BUSY.has(state.phase)));
    panel.dataset.updatePhase = state.phase;
  };

  // 必须按**具名属性**传 `changed`：createUpdateController 解构的是单个对象，
  // 之前写成 `(依赖对象, render)` 会把 render 当成第二个位置参数丢掉——界面仍然
  // 挂载成功、controller 状态也对，只是永远不重绘 DOM，非常难查。
  controller = createUpdateController({
    check: async () => {
      const result = await invoke('update_check');
      const skippedSettings = await invoke('update_skipped_version').then(
        version => ({ ok: true, version }),
        error => ({ ok: false, error: errorText(error, '请重新进入设置页重试') }),
      );
      return { ...result, skippedSettings };
    },
    install: () => invoke('update_install'),
    restart: () => invoke('update_restart'),
    dismissRejection: () => invoke('update_dismiss_rejection'),
    skipVersion: () => invoke('update_skip_version'),
    restoreSkippedVersion: () => invoke('update_restore_skipped_version'),
    changed: render,
    initialPhase: 'restoring',
  });
  panel.__updateController = controller;

  let unlisten = null;
  let subscribePendingGeneration = null;
  let monitor = null;
  let monitorBusyGeneration = null;
  let detached = false;
  let attached = false;
  let attachmentGeneration = 0;
  let observer = null;
  const subscribe = async () => {
    const generation = attachmentGeneration;
    if (unlisten || detached) return;
    if (subscribePendingGeneration === generation) return;
    subscribePendingGeneration = generation;
    try {
      const stop = await listen('update://progress', message => {
        // Tauri 的 listen 回调收到的是 `{event, id, payload}`，进度数据在 **payload** 里。
        // 写成 `message.downloaded` 的话取到 undefined，进度条永远停在「下载中…」——
        // 而且不抛错、测试要靠真实事件才看得出来。
        const payload = message?.payload ?? message;
        controller.progress(payload?.downloaded, payload?.total);
      });
      if (detached || generation !== attachmentGeneration) stop();
      else unlisten = stop;
    } finally {
      if (subscribePendingGeneration === generation) subscribePendingGeneration = null;
    }
  };
  const monitorSession = generation => {
    if (monitor?.generation === generation) return;
    const timer = setInterval(async () => {
      if (monitorBusyGeneration === generation) return;
      monitorBusyGeneration = generation;
      try {
        const session = await invoke('update_session_status');
        if (detached || generation !== attachmentGeneration) return;
        controller.restoreSessionStatus(session);
        if (!['checking', 'downloading'].includes(controller.state.phase)) {
          clearInterval(timer);
          if (monitor?.timer === timer) monitor = null;
        }
      } catch {
        if (detached || generation !== attachmentGeneration) return;
        controller.restoreSessionStatus(null);
        clearInterval(timer);
        if (monitor?.timer === timer) monitor = null;
      } finally {
        if (monitorBusyGeneration === generation) monitorBusyGeneration = null;
      }
    }, 1000);
    monitor = { generation, timer };
  };
  const watchRemoval = () => {
    if (observer || typeof MutationObserver !== 'function' || !document.documentElement) return;
    observer = new MutationObserver(() => {
      if (!panel.isConnected) detach();
    });
    observer.observe(document.documentElement, { childList: true, subtree: true });
  };
  const resume = () => {
    if (attached || !panel.isConnected) return;
    if (detached) controller.beginSessionRestore();
    detached = false;
    attached = true;
    const generation = ++attachmentGeneration;
    watchRemoval();
    Promise.allSettled([
      invoke('update_session_status'),
      invoke('update_skipped_version').then(
        version => ({ version }),
        error => ({ error }),
      ),
    ])
      .then(([sessionResult, skippedResult]) => {
        if (detached || generation !== attachmentGeneration) return;
        const session = sessionResult.status === 'fulfilled' ? sessionResult.value : null;
        const skipped = skippedResult.status === 'fulfilled' ? skippedResult.value : { error: skippedResult.reason };
        if (skipped.error) {
          controller.setNotice(`读取跳过版本设置失败：${errorText(skipped.error, '请重新进入设置页重试')}`);
        } else {
          controller.setSkippedVersion(skipped.version);
        }
        controller.restoreSessionStatus(session);
        if (['checking', 'downloading'].includes(session?.phase)) {
          subscribe().catch(() => {});
          monitorSession(generation);
        }
      })
      .catch(() => {
        if (!detached && generation === attachmentGeneration) controller.restoreSessionStatus(null);
      });
  };
  const detach = () => {
    if (detached) return;
    detached = true;
    attached = false;
    attachmentGeneration += 1;
    unlisten?.();
    unlisten = null;
    if (monitor) clearInterval(monitor.timer);
    monitor = null;
    observer?.disconnect();
    observer = null;
  };
  panel.addEventListener('update:detach', detach);
  panel.addEventListener('update:resume', resume);

  find('check').addEventListener('click', () => { subscribe(); controller.check(); });
  find('install').addEventListener('click', () => controller.install());
  find('skip').addEventListener('click', () => controller.skipCurrentVersion());
  find('restore-skip').addEventListener('click', () => controller.restoreSkippedVersion());
  find('restart').addEventListener('click', () => controller.restart());
  find('dismiss').addEventListener('click', () => {
    if (controller.state.phase === 'rejected') controller.closeRejection();
    else controller.dismiss();
  });

  // **先渲染一次初始状态**。changed 回调只在状态**变化**时触发，而初始状态
  // 从未被谁改动过——不主动渲染的话，面板会一直是空白：无版本号、无文案、
  // 连 `data-update-phase` 都没有，测试与样式都无从下手。
  render(controller.state);

  // 当前版本只读本机包信息，不出网——所以挂载时就可以显示。
  invoke('update_current_version')
    .then(version => controller.setCurrentVersion(version))
    .catch(() => {});
  resume();
  return controller;
}

export { formatBytes, updateStatusText };
