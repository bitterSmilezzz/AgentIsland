import { invoke, listen } from './tauri.js';
import { createUpdateController, updateStatusText, formatBytes } from './update-state.js';

const BUSY = new Set(['checking', 'downloading', 'restarting']);
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

/** 「检查更新」分区。**默认不检查**——渲染出来时状态是「尚未检查更新」。
 *  容器带 `data-update` 而不是 `data-update-root`：项目里已有一个 `data-draft-ignore`
 *  之类的约定，这里用独立前缀避免与其它模块的选择器冲突。 */
export function updatePanelHtml() {
  return `<section class="settings-section update-section" data-update-panel>
    <h2>更新</h2>
    <p class="update-note">仅在你点击时检查与安装更新；安装完成后需你再次确认才会重启。</p>
    <p class="update-current" data-update-current></p>
    <div class="update-actions">
      <button type="button" class="mini-btn" data-update-check>检查更新</button>
      <button type="button" class="mini-btn" data-update-install hidden>下载并安装</button>
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
    find('check').hidden = state.phase !== 'idle' && state.phase !== 'failed';
    find('check').disabled = BUSY.has(state.phase);
    find('check').textContent = state.phase === 'failed' ? '重试' : '检查更新';
    const install = find('install');
    install.hidden = state.phase !== 'available';
    install.disabled = BUSY.has(state.phase);
    install.textContent = state.phase === 'downloading'
      ? (state.total ? `${Math.min(100, Math.floor((state.downloaded ?? 0) / state.total * 100))}%` : '下载中…')
      : '下载并安装';
    const restart = find('restart');
    restart.hidden = state.phase !== 'ready';
    restart.disabled = BUSY.has(state.phase);
    const dismiss = find('dismiss');
    dismiss.hidden = !['available', 'failed', 'uptodate'].includes(state.phase);
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
    check: () => invoke('update_check'),
    install: () => invoke('update_install'),
    restart: () => invoke('update_restart'),
    changed: render,
  });
  panel.__updateController = controller;

  let unlisten = null;
  const subscribe = async () => {
    if (unlisten) return;
    unlisten = await listen('update://progress', message => {
      // Tauri 的 listen 回调收到的是 `{event, id, payload}`，进度数据在 **payload** 里。
      // 写成 `message.downloaded` 的话取到 undefined，进度条永远停在「下载中…」——
      // 而且不抛错、测试要靠真实事件才看得出来。
      const payload = message?.payload ?? message;
      controller.progress(payload?.downloaded, payload?.total);
    });
  };
  panel.addEventListener('update:detach', () => { unlisten?.(); unlisten = null; });

  find('check').addEventListener('click', () => { subscribe(); controller.check(); });
  find('install').addEventListener('click', () => controller.install());
  find('restart').addEventListener('click', () => controller.restart());
  find('dismiss').addEventListener('click', () => controller.dismiss());

  // **先渲染一次初始状态**。changed 回调只在状态**变化**时触发，而初始状态
  // 从未被谁改动过——不主动渲染的话，面板会一直是空白：无版本号、无文案、
  // 连 `data-update-phase` 都没有，测试与样式都无从下手。
  render(controller.state);

  // 当前版本只读本机包信息，不出网——所以挂载时就可以显示。
  invoke('update_current_version')
    .then(version => controller.setCurrentVersion(version))
    .catch(() => {});
  return controller;
}

export { formatBytes, updateStatusText };
