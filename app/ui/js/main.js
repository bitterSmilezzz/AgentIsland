// AgentIsland 前端入口：状态管理、贴边交互、渲染调度
import {
  hydrateProvider,
  hydrateReport,
  hydrateTodo,
  renderCard,
  renderSidebar,
  renderSidebarDetail,
  renderSliver,
  sliverSize,
} from './views.js';
import { invoke } from './tauri.js';
import { isSidebar, SHELL } from './shell.js';

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
    if (isSidebar()) {
      if (state.route === 'list') renderSidebar();
      return;
    }
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

  // 取不到设置时的兜底：**与 Rust `Settings::default()` 逐字段一致**。
  // 少写一个字段，设置页那一栏就会是空的——而「空」在界面上读起来像「用户没设过」，
  // 于是他会在一个其实生效着的值上反复调。
  state.settings = await invoke('get_settings').catch(() => ({
    appearance: 'system', shell_mode: 'island', sidebar_edge: 'right',
    dock_edge: 'top', dock_anchor: 0.5,
    collapse_delay: 0.5, sample_interval: 2, idle_sample_interval: 5,
    working_window: 60, min_working_hold: 10, active_session_window: 600,
    runaway_cpu_alert: true, runaway_cpu_threshold: 70, runaway_duration_threshold: 300,
    cpu_threshold: 6, battery_saver_enabled: true,
    token_alert_enabled: true, token_alert_threshold: 200000,
    daily_token_budget: 0, budget_alert_enabled: true,
    auto_anomalies_alert: true,
    notification_policy: 'standard', play_completion_sound: true, disabled_agents: [],
    launch_at_login: false, hide_docked_sliver: false, compact_view: false,
    global_hot_key_enabled: true, menu_bar_badge_mode: 'iconOnly',
    screen_follow_mode: 'followMouse',
  }));
  // 归一化：Rust/手动改配置可能出现 'Top'/'Dark' 等大小写变体
  state.settings.dock_edge = (state.settings.dock_edge ?? 'top').toLowerCase();
  state.settings.appearance = (state.settings.appearance ?? 'system').toLowerCase();
  applyAppearance(state.settings.appearance);

  // 形态分派：两个窗口加载同一个 index.html，靠 URL 上的 `?shell=` 分辨自己是谁。
  // 侧边栏分支**不再做灵动岛那套事**（细条几何、贴边放置、展开收起、失焦防抖）——
  // 那些只为「浮在屏幕边沿的一小块」而存在。
  // 形态类由 index.html 的内联脚本在第一次绘制前挂好；这里只把它写进日志——
  // 「收拢的样式到底生效了没有」取决于这个类，而它是运行时事实，值得留一行证据。
  invoke('log_from_ui', {
    message: `shell=${document.documentElement.className || '(未设置)'}`,
  }).catch(() => {});

  const { listen } = await import('./tauri.js');
  if (isSidebar()) {
    await invoke('place_sidebar').catch(() => {});
    // 启动路由也认（与灵动岛同一条约定：`--route=provider` 这类参数由托盘/命令行走）。
    // 侧边栏原先不认它，于是「用一个参数直接开到某一页」在两种形态下行为不一致。
    if (state.bootRoute) state.route = state.bootRoute;
    // 角标要显示未完成数，所以启动时读一次待办（**只读计数**，不读页面内容——
    // 读页面内容会白白重画一次，还会和下面的 hydrate 抢同一块 DOM）
    const todos = await invoke('todos_list').catch(() => null);
    state.todosPending = todos?.pending ?? 0;
    renderSidebar();
    // 数据型页面进来要填一次内容（与点导航时同一路径，避免两套入口两种行为）
    if (state.route === 'tokenAnalytics') await hydrateReport();
    if (state.route === 'provider') await hydrateProvider();
    if (state.route === 'todo') await hydrateTodo();
    // 详情页：先铺壳再填内容（与灵动岛同一个页面函数，只是外壳不同）
    if (state.route.startsWith('agentDetail:')) {
      renderSidebarDetail();
      await hydrateReport();
    }
    await listen('engine://tick', (e) => {
      state.engine = e.payload;
      // **只有「实时列表」这一页随推送重画**。分析页、档位页是「进来时渲染一次」：
      // 每 2 秒重画一次会把它们打回「加载中」，档位页还会把用户正在填的表单冲掉
      // （同一个坑在分析页上也踩过一次）。
      if (state.route === 'list') renderSidebar();
    });
    // 侧边栏没有「展开/收起」：它一直是展开的
    return;
  }

  applyEdge();
  renderSliver();

  const size = sliverSize(state.settings.dock_edge);
  await invoke('place_island', { width: size.w, height: size.h });

  // 引擎推送
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
