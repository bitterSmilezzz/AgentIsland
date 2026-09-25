// AgentIsland 前端入口：状态管理、贴边交互、渲染调度
import { renderCard, renderSliver, sliverSize } from './views.js';
import { invoke } from './tauri.js';

const $root = () => document.getElementById('root');

const state = {
  expanded: false,
  engine: null,       // EngineState（Rust 推送）
  settings: null,
  route: 'list',      // list | tokenAnalytics | agentDetail:<id>
  demo: false,
  bootRoute: '',
  collapseTimer: null,
  lastEventDetail: null,
};

export const getState = () => state;
export const setState = (patch) => Object.assign(state, patch);

// MARK: 主题

export function applyAppearance(mode) {
  const m = (mode ?? 'system').toLowerCase();
  const dark = m === 'dark' || (m !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.classList.toggle('theme-dark', dark);
  document.documentElement.classList.toggle('theme-light', !dark);
}

// MARK: 布局

export function edgeClass() {
  const edge = state.settings?.dock_edge ?? 'top';
  return `edge-${edge}`;
}

export function applyEdge() {
  const root = $root();
  root.className = edgeClass();
}

// MARK: 展开 / 收起

export async function expand() {
  if (state.expanded) return;
  state.expanded = true;
  clearTimeout(state.collapseTimer);

  const root = $root();
  applyEdge();
  // 先渲染内容（隐藏态量高；解除 max-height 避免被旧窗口高度钳住）
  renderCard();
  const card = root.querySelector('.card');
  if (card) {
    card.style.visibility = 'hidden';
    card.style.maxHeight = 'none';
  }

  const h = Math.min(card ? card.getBoundingClientRect().height : 480, 520);
  const w = 330;
  const cs = card ? getComputedStyle(card) : null;
  const rs = getComputedStyle(root);

  await invoke('place_island', { width: w, height: h });
  if (card) {
    card.style.maxHeight = '';
    card.style.visibility = '';
    card.classList.add('open');
  }
  // 收起计时只由「光标离开」触发（见 renderCard 绑定的 mouseleave），不在展开时武装
}

export async function collapse() {
  if (!state.expanded) return;
  state.expanded = false;
  clearTimeout(state.collapseTimer);
  state.route = 'list';
  const root = $root();
  applyEdge();
  renderSliver();
  const size = sliverSize(state.settings?.dock_edge ?? 'top');
  await invoke('place_island', { width: size.w, height: size.h });
}

export function armCollapseTimer() {
  clearTimeout(state.collapseTimer);
  const delay = (state.settings?.collapse_delay ?? 0.5) * 1000;
  state.collapseTimer = setTimeout(() => {
    if (state.expanded && !document.querySelector(':hover')) collapse();
  }, Math.max(300, delay));
}

// MARK: 渲染调度

let rafPending = false;
export function scheduleRender() {
  if (rafPending) return;
  rafPending = true;
  requestAnimationFrame(async () => {
    rafPending = false;
    if (state.expanded) {
      if (state.route === 'list') {
        renderCard();
      }
      await resizeToContent();
    } else {
      renderSliver();
    }
  });
}

/// 按卡片内容自适应窗口高度（引擎数据变化后窗口跟随）
export async function resizeToContent() {
  const card = document.querySelector('.card');
  if (!card) return;
  card.style.maxHeight = 'none';
  const h = Math.min(card.getBoundingClientRect().height, 520);
  card.style.maxHeight = '';
  if (Math.abs(window.innerHeight - h) > 6) {
    await invoke('place_island', { width: 330, height: h }).catch(() => {});
  }
}

// MARK: 启动

async function boot() {
  const boot = await invoke('get_boot_args').catch(() => ({ demo: false, expand: false, route: '' }));
  state.demo = !!boot.demo;
  state.bootRoute = boot.route || '';

  state.settings = await invoke('get_settings').catch(() => ({
    appearance: 'system', dock_edge: 'top', dock_anchor: 0.5,
    collapse_delay: 0.5, sample_interval: 2, cpu_threshold: 6,
    token_alert_enabled: true, token_alert_threshold: 200000,
    notification_policy: 'standard', play_completion_sound: true, disabled_agents: [],
  }));
  // 归一化：Rust/手动改配置可能出现 'Top'/'Dark' 等大小写变体
  state.settings.dock_edge = (state.settings.dock_edge ?? 'top').toLowerCase();
  state.settings.appearance = (state.settings.appearance ?? 'system').toLowerCase();
  applyAppearance(state.settings.appearance);
  applyEdge();
  renderSliver();

  const size = sliverSize(state.settings.dock_edge);
  await invoke('place_island', { width: size.w, height: size.h });

  // 引擎推送
  const { listen } = await import('./tauri.js');
  await listen('engine://tick', (e) => {
    state.engine = e.payload;
    scheduleRender();
  });
  await listen('tray://toggle', () => (state.expanded ? collapse() : expand()));

  if (boot.expand) {
    setTimeout(async () => {
      await expand();
      const r = state.bootRoute;
      if (r === 'tokenAnalytics' || r.startsWith('agentDetail:')) {
        state.route = r;
        renderCard();
        await resizeToContent();
      }
    }, 1500);
  }
}

// 失焦收起：窗口尺寸/位置变化会引起焦点抖动（resize 期间 focus/blur 对），不可靠；
// 收起统一交给「光标离开卡片」防抖（mouseleave → armCollapseTimer），与 macOS 端一致。

// 调试：未捕获错误写入日志文件（经 Rust 落盘）
addEventListener('error', (e) => { invoke('log_from_ui', { message: 'ERR ' + e.message + ' @' + e.filename + ':' + e.lineno }).catch(() => {}); });
addEventListener('unhandledrejection', (e) => {
  invoke('log_from_ui', { message: 'REJ ' + (e.reason?.message ?? String(e.reason)) }).catch(() => {});
});

boot();
