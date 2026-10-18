// 自动更新流程的**状态机**：纯逻辑、不碰 DOM，所以能直接被 node 测。
//
// 三条硬规则必须落在这里，而不是散在界面事件里：
//   1. 只有 `check()` 会检查更新；
//   2. 只有 `install()` 且当前处于 `available` 才安装——没有「自动安装」这条路径；
//   3. 只有 `restart()` 且当前处于 `ready` 才重启——**安装完不自动重启**。
//
// 测试要能证明「安装成功后仍在 ready、重启函数一次都没被调用」，否则
// 「不在用户不知情时重启」就只是一句注释。

const BUSY_PHASES = ['checking', 'downloading', 'restarting'];

/**
 * 取出可直接展示的错误文案。
 *
 * 后端 `invoke` 成功时返回字符串、失败时也可能抛 `Error` 对象——只认 `typeof ===
 * 'string'` 会把真实原因（如「更新包签名校验失败」）换成一句通用兜底，
 * 用户就看不到为什么被拒。所以两种都要读，取不到才用兜底。
 * 兜底必须存在：绝不把 `[object Object]` 之类的东西显示给用户。
 */
export function errorText(error, fallback) {
  const raw = typeof error === 'string' ? error : (error && typeof error.message === 'string' ? error.message : '');
  return raw.trim() ? raw : fallback;
}

export function createUpdateController({ check, install, restart, changed = () => {} } = {}) {
  const state = {
    phase: 'idle',
    currentVersion: '',
    version: '',
    notes: '',
    date: '',
    downloaded: null,
    total: null,
    reason: '',
  };
  const notify = () => changed(controller.state);
  const fail = (error, fallback) => {
    state.phase = 'failed';
    state.reason = errorText(error, fallback);
  };

  const controller = {
    get state() {
      return { ...state };
    },
    get busy() {
      return BUSY_PHASES.includes(state.phase);
    },
    get canInstall() {
      return state.phase === 'available';
    },
    get canRestart() {
      return state.phase === 'ready';
    },
    setCurrentVersion(version) {
      if (version == null || version === '') return;
      state.currentVersion = String(version);
      notify();
    },
    async check() {
      if (controller.busy || typeof check !== 'function') return;
      state.phase = 'checking';
      state.reason = '';
      notify();
      try {
        const result = await check();
        if (!result || typeof result.available !== 'boolean') throw new Error('invalid');
        if (result.currentVersion) state.currentVersion = String(result.currentVersion);
        if (result.available) {
          state.phase = 'available';
          state.version = String(result.version ?? '');
          state.notes = String(result.notes ?? '');
          state.date = String(result.date ?? '');
        } else {
          state.phase = 'uptodate';
          state.version = '';
          state.notes = '';
          state.date = '';
        }
      } catch (error) {
        fail(error, '检查更新失败，请稍后重试');
      } finally {
        notify();
      }
    },
    async install() {
      // 关键：只有 available 才允许安装。idle / uptodate / failed / ready 一律不动。
      if (!controller.canInstall || typeof install !== 'function') return;
      state.phase = 'downloading';
      state.downloaded = null;
      state.total = null;
      state.reason = '';
      notify();
      try {
        await install();
        // 安装完成**不**调用 restart()：等用户点「立即重启」。
        state.phase = 'ready';
      } catch (error) {
        fail(error, '下载或安装失败，请重试');
      } finally {
        notify();
      }
    },
    progress(downloaded, total) {
      if (state.phase !== 'downloading') return;
      const done = Number(downloaded);
      state.downloaded = Number.isFinite(done) && done >= 0 ? done : null;
      const size = Number(total);
      state.total = Number.isFinite(size) && size > 0 ? size : null;
      notify();
    },
    async restart() {
      if (!controller.canRestart || typeof restart !== 'function') return;
      state.phase = 'restarting';
      notify();
      try {
        await restart();
      } catch (error) {
        state.phase = 'ready';
        state.reason = errorText(error, '重启失败，请手动重启应用');
        notify();
      }
    },
    dismiss() {
      if (!['available', 'failed', 'uptodate'].includes(state.phase)) return;
      state.phase = 'idle';
      state.reason = '';
      notify();
    },
  };
  return controller;
}

export function formatBytes(bytes) {
  const value = Number(bytes);
  if (!Number.isFinite(value) || value < 0) return '—';
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}

/// 状态 → 一句话。空字符串表示「没有可显示的进度」。
export function updateStatusText(state) {
  switch (state?.phase) {
    case 'checking':
      return '正在检查更新…';
    case 'uptodate':
      return '已是最新版本';
    case 'available':
      return state.version ? `发现新版本 v${state.version}` : '发现新版本';
    case 'downloading':
      if (state.downloaded == null) return '正在下载更新包…';
      if (state.total == null) return `已下载 ${formatBytes(state.downloaded)}…`;
      return `正在下载 ${Math.min(100, Math.floor((state.downloaded / state.total) * 100))}%（${formatBytes(state.downloaded)} / ${formatBytes(state.total)}）`;
    case 'ready':
      return '更新已安装，重启后生效';
    case 'restarting':
      return '正在重启…';
    case 'failed':
      return state.reason || '更新失败，请稍后重试';
    default:
      return '尚未检查更新';
  }
}
