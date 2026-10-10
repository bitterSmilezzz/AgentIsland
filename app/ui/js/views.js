import { rememberPage, pageMotion } from './page-motion.js';
import { trendCardHtml, bindTrend, hourRecords } from './usage-trend.js';
import { toolBudgetsHtml, bindToolBudgets } from './tool-budgets.js';
import { usageContextHtml, bindUsageContext } from './usage-context.js';
import { reportPanelHtml, bindReport, visibleReportPanel, openUsageReport } from './report-panel.js';
import { pageSessions, hydrateSessions, refreshSessions } from './sessions-page.js';
import { bindQuickNavigation } from './quick-navigation.js';
import { workbenchPages } from './navigation.js';
import { pagePrompts, hydratePrompts } from './prompts-page.js';
import { workspaceFlow, workspaceRoute, workspaceReceiptLabel, workspaceWritable } from './workspace-flow.js';
import { getTaskAttention, taskAttentionHtml, bindTaskAttention, taskCoversEvent, taskCoversSnapshot } from './task-attention.js';
import { pageTasks, hydrateTasks, refreshTasks, selectTaskProject, selectTask } from './tasks-page.js';
import { pageWindowLayout, hydrateWindowLayout, selectLayoutRule, selectLayoutRecovery } from './window-layout-page.js';
import { pageWorkspaces, hydrateWorkspaces } from './workspaces-page.js';
import { modelDirectoryHtml, bindModelWorkspace, bindModelDirectory } from './models-page.js';
import { pageConnections, hydrateConnections } from './connections-page.js';
import { pageMcp, hydrateMcp } from './mcp-page.js';
import {pageClaudePlan,hydrateClaudePlan} from './claude-plan-page.js';
import { PageCache, pageRequest, revealNavigationItem, previewFocusTarget } from './page-host.js';
import { beginProviderFeedback, showProviderFeedback } from './provider-feedback.js';
import { healthReason, agentNextStep } from './agent-actions.js';
// 灵动岛视图渲染（IslandView / AgentRowView / TokenSummaryBar / SubViews 的 Web 对应物）
import { invoke } from './tauri.js';
import { agentIcon } from './agent-icons.js';
import { isIsland, isWorkbench } from './shell.js';
import { captureNavigation, stageNavigation, navigationActive, focusNavigation } from './island-navigation.js';
import { getState, setState, saveSettings, expand, collapse, armCollapseTimer, scheduleRender, resizeToContent, applyEdge, applyLayout, retainWorkbenchControls } from './main.js';

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

const workbenchScroll = new Map();
const workbenchPagesCache = new PageCache();

// MARK: Token 格式化（与 Rust/Win 端同口径）

export function compact(tokens) {
  const n = tokens;
  if (tokens < 0) return '0';
  if (n < 1e3) return `${tokens}`;
  if (n < 1e6) return trimZero((n / 1e3).toFixed(1)) + 'k';
  if (n < 1e9) {
    const m = n / 1e6;
    return (m >= 100 ? m.toFixed(0) : trimZero(m.toFixed(2))) + 'M';
  }
  return trimZero((n / 1e9).toFixed(2)) + 'G';
}
function trimZero(s) { return s.replace(/\.?0+$/, ''); }
// 成本文案。`estimated` 为真时带 `~`——它是「这个数是估的」唯一的痕迹，
// 按数字重排不许把它弄丢（与 Swift TokenCostEstimator.formatEstimate 同口径）。
export function costText(c, estimated = false) { return c > 0.005 ? `${estimated ? '~' : ''}$${c.toFixed(2)}` : ''; }

/// 事件摘要文案（Rust 端方法不随 JSON 序列化，与 AgentTaskEvent::summary 同口径）
function eventSummary(ev) {
  if (ev.message) return ev.message;
  if (ev.event_type === 'attention') return `${ev.agent_name} 等待确认操作`;
  if (ev.event_type === 'costSpike') return `⚠️ ${ev.agent_name} 资源/Token 消耗突增`;
  return `${ev.agent_name} 任务完成`;
}

/// Attention status already lives in the header; preserve the actual requested action.
export function bannerTitle(ev) {
  if (ev.event_type === 'attention') {
    return String(ev.message ?? '').trim().replace(/^(?:(?:需要确认|等待确认操作|等待确认|待确认)[：:\s]*)+/u, '').trim() || '等待你的确认';
  }
  const prefix = ev.event_type === 'completed' ? '已完成' : '告警';
  const msg = eventSummary(ev);
  return msg.startsWith(prefix) ? msg : `${prefix}：${msg}`;
}

// MARK: 状态 → 颜色（ActivityLevel 色阶，主题切换由 CSS 变量承担）

function levelColors(level) {
  const dark = document.documentElement.classList.contains('theme-dark');
  switch (level) {
    case 'working':
    case 'completed':
      return dark
        ? { fg: 'var(--working)', bg: 'rgba(48,209,88,0.12)', border: 'rgba(48,209,88,0.32)' }
        : { fg: '#047857', bg: '#ecfdf5', border: '#a7f3d0' };
    case 'attention':
      return dark
        ? { fg: 'var(--warning)', bg: 'rgba(255,149,0,0.12)', border: 'rgba(255,149,0,0.32)' }
        : { fg: '#b45309', bg: '#fffbeb', border: '#fde68a' };
    case 'idle':
      return dark
        ? { fg: 'var(--idle)', bg: 'rgba(255,214,10,0.12)', border: 'rgba(255,214,10,0.32)' }
        : { fg: '#475569', bg: '#f1f5f9', border: '#e2e8f0' };
    default:
      return dark
        ? { fg: 'var(--offline)', bg: 'rgba(142,142,147,0.12)', border: 'rgba(142,142,147,0.32)' }
        : { fg: '#64748b', bg: '#f8fafc', border: '#e2e8f0' };
  }
}

// MARK: 环形微仪表盘（AgentRingView）

function ringSvg(size, level, cpu, progress, ringColor) {
  const stroke = 3;
  const inset = stroke / 2 + 0.5;
  const w = size - stroke - 1;
  const r = w * 0.3;
  // 圆角矩形路径（起点 = 顶边左端，顺时针）
  const path = `M ${inset + r} ${inset} H ${inset + w - r} A ${r} ${r} 0 0 1 ${inset + w} ${inset + r} V ${inset + w - r} A ${r} ${r} 0 0 1 ${inset + w - r} ${inset + w} H ${inset + r} A ${r} ${r} 0 0 1 ${inset} ${inset + w - r} V ${inset + r} A ${r} ${r} 0 0 1 ${inset + r} ${inset} Z`;
  const straight = w - 2 * r;
  const perimeter = 2 * straight + 2 * Math.PI * r;
  const startDist = r + straight / 2; // 顶边中点（弧线起点）
  const arcLen = Math.max(0.002, progress) * perimeter;

  let inner = '';
  if (level === 'working') {
    const spinR = (w - 2 * r) / 2 - 3.5;
    const spinC = 2 * Math.PI * spinR;
    inner = `<circle class="spin-arc" cx="${size / 2}" cy="${size / 2}" r="${spinR}"
      fill="none" stroke="${ringColor}" stroke-width="${Math.max(1.6, stroke * 0.6)}"
      stroke-linecap="round" stroke-dasharray="${(spinC * 0.32).toFixed(1)} ${spinC.toFixed(1)}"/>`;
  } else if (level === 'attention') {
    const spinR = (w - 2 * r) / 2 - 3.5;
    const spinC = 2 * Math.PI * spinR;
    inner = `<circle class="breath-arc" cx="${size / 2}" cy="${size / 2}" r="${spinR}"
      fill="none" stroke="var(--ring-yellow)" stroke-width="${Math.max(1.5, stroke * 0.55)}"
      stroke-linecap="round" stroke-dasharray="${(spinC * 0.85).toFixed(1)} ${spinC.toFixed(1)}"/>`;
  }

  return `<svg width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">
    <path d="${path}" fill="var(--row-bg)" stroke="var(--ring-track)" stroke-width="${stroke}"/>
    ${progress > 0 ? `<path d="${path}" fill="none" stroke="${ringColor}" stroke-width="${stroke}"
      stroke-linecap="round" stroke-dasharray="${arcLen.toFixed(1)} ${(perimeter - arcLen).toFixed(1)}"
      stroke-dashoffset="${(-startDist).toFixed(1)}"/>` : ''}
    ${inner}
  </svg>`;
}

function ringHtml(snap, size) {
  let progress = 0;
  let color = 'var(--ring-track)';
  const dark = document.documentElement.classList.contains('theme-dark');
  if (snap.level === 'working') {
    progress = Math.max(0.5, Math.min(0.5 + (snap.cpu_percent ?? 0) / 100 * 0.5, 1));
    color = snap.is_hung === true ? 'var(--ring-red)' : (snap.cpu_percent ?? 0) >= 80 ? 'var(--ring-orange)' : 'var(--ring-green)';
  } else if (snap.level === 'attention') {
    progress = 0.78; color = 'var(--ring-yellow)';
  } else if (snap.level === 'completed') {
    progress = 0.45; color = 'var(--ring-green)';
  } else if (snap.level === 'idle' && snap.token_usage && snap.token_usage.tokens24h > 0) {
    progress = Math.max(0.15, Math.min(snap.token_usage.tokens24h / 200000, 1));
    color = dark ? '#28e07b' : '#157f3c';
  }
  return `<div class="ring" data-ring>${ringSvg(size, snap.level, snap.cpu_percent, progress, color)}
    <div class="glyph">${agentIcon(snap, 'agent-avatar--bare')}</div></div>`;
}

// MARK: 小图标（内联 SVG）

const ICONS = {
  search: '<svg viewBox="0 0 16 16"><path d="M11.7 10.3a5 5 0 1 0-1.4 1.4l3 3 1.4-1.4-3-3zM7 10a3 3 0 1 1 0-6 3 3 0 0 1 0 6z"/></svg>',
  moon: '<svg viewBox="0 0 16 16"><path d="M6 2a6.5 6.5 0 1 0 8 8A7 7 0 0 1 6 2z"/></svg>',
  sun: '<svg viewBox="0 0 16 16"><path d="M8 5a3 3 0 1 0 0 6 3 3 0 0 0 0-6zm0-3h0v2m0 10v2m6-7h2M0 8h2m9.7-4.7 1.4-1.4M2.9 13.1l1.4-1.4m7.4 0 1.4 1.4M2.9 2.9l1.4 1.4" stroke="currentColor" stroke-width="1.3" fill="none"/><circle cx="8" cy="8" r="3"/></svg>',
  wrench: '<svg viewBox="0 0 16 16"><path d="M13.7 4.3a4 4 0 0 1-5.3 5.1L4 13.8a1.5 1.5 0 0 1-2.1-2.1l4.4-4.4a4 4 0 0 1 5-5.3L9 4.3 10.4 7l2.9-2.4z"/></svg>',
  clock: '<svg viewBox="0 0 16 16"><path d="M8 1a7 7 0 1 0 0 14A7 7 0 0 0 8 1zm.7 3v4.2l3 1.8-.7 1.2-3.7-2.2V4h1.4z"/></svg>',
  chevUp: '<svg viewBox="0 0 16 16"><path d="M8 5.5 13 10.5 11.6 12 8 8.4 4.4 12 3 10.5z"/></svg>',
  chevDown: '<svg viewBox="0 0 16 16"><path d="M8 10.5 3 5.5 4.4 4 8 7.6 11.6 4 13 5.5z"/></svg>',
  chevLeft: '<svg viewBox="0 0 16 16"><path d="M5.5 8 10.5 3 12 4.4 8.4 8 12 11.6 10.5 13z"/></svg>',
  chevRight: '<svg viewBox="0 0 16 16"><path d="M10.5 8 5.5 13 4 11.6 7.6 8 4 4.4 5.5 3z"/></svg>',
  close: '<svg viewBox="0 0 16 16"><path d="M4.4 3 8 6.6 11.6 3 13 4.4 9.4 8l3.6 3.6-1.4 1.4L8 9.4 4.4 13 3 11.6 6.6 8 3 4.4z"/></svg>',
  hand: '<svg viewBox="0 0 16 16"><path d="M7 2v6H6V3.5a.75.75 0 0 0-1.5 0V10l-1-1.3a1.1 1.1 0 0 0-1.7 1.4l3.2 4A3 3 0 0 0 7.4 15H9a4 4 0 0 0 4-4V6.5a.75.75 0 0 0-1.5 0V9h-.5V4.5a.75.75 0 0 0-1.5 0V9H9V2.75A.75.75 0 0 0 8.25 2 1.25 1.25 0 0 0 7 3.25z"/></svg>',
  terminal: '<svg viewBox="0 0 16 16"><path d="M2 3h12v10H2V3zm1.5 1.5v7h9v-7h-9zM4.8 6l1.8 2-1.8 2 1 1 2.6-3L5.8 5l-1 1z"/></svg>',
  jump: '<svg viewBox="0 0 16 16"><path d="M4 3h4v1.5H5.5v6h6V8H13v4H3V3h1z"/><path d="M9 3h4v4h-1.5V5.6L8 9.1 7 8.1l3.5-3.6H9V3z"/></svg>',
  warn: '<svg viewBox="0 0 16 16"><path d="M8 1.5 15 14H1L8 1.5zM7.3 6v4h1.4V6H7.3zm.7 7a.9.9 0 1 0 0-1.8.9.9 0 0 0 0 1.8z"/></svg>',
};

// A single stroke weight and optical box for the island toolbar.
function islandToolbarIcon(name) {
  const paths = {
    search: '<circle cx="10.5" cy="10.5" r="6"/><path d="m15 15 4.5 4.5"/>',
    settings: '<path d="M4 7h3m4 0h9M4 17h9m4 0h3"/><circle cx="9" cy="7" r="2"/><circle cx="15" cy="17" r="2"/>',
    top: '<path d="m7 14 5-5 5 5"/>',
    bottom: '<path d="m7 10 5 5 5-5"/>',
    left: '<path d="m14 7-5 5 5 5"/>',
    right: '<path d="m10 7 5 5-5 5"/>',
  };
  return `<svg class="island-toolbar-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] ?? paths.top}</svg>`;
}

// MARK: 贴边反向倒角形状（curl 10 / 圆角 18，四边互为镜像）

export function notchPathD(w, h, edge) {
  const c = 18, k = 10; // corner / curl（与 IslandMetrics 同源）
  if (edge === 'top') {
    // 屏幕边在顶部：上侧两角为反向倒角（concave），下侧两角为外圆角
    return `M 0 0 A ${k} ${k} 0 0 1 ${k} ${k} L ${k} ${h - c} A ${c} ${c} 0 0 0 ${k + c} ${h} L ${w - k - c} ${h} A ${c} ${c} 0 0 0 ${w - k} ${h - c} L ${w - k} ${k} A ${k} ${k} 0 0 1 ${w} 0 Z`;
  }
  if (edge === 'bottom') {
    return `M 0 ${h} A ${k} ${k} 0 0 0 ${k} ${h - k} L ${k} ${c} A ${c} ${c} 0 0 1 ${k + c} 0 L ${w - k - c} 0 A ${c} ${c} 0 0 1 ${w - k} ${c} L ${w - k} ${h - k} A ${k} ${k} 0 0 0 ${w} ${h} Z`;
  }
  if (edge === 'right') {
    return `M ${w} 0 A ${k} ${k} 0 0 0 ${w - k} ${k} L ${c} ${k} A ${c} ${c} 0 0 0 0 ${k + c} L 0 ${h - k - c} A ${c} ${c} 0 0 0 ${c} ${h - k} L ${w - k} ${h - k} A ${k} ${k} 0 0 0 ${w} ${h} Z`;
  }
  // left
  return `M 0 0 A ${k} ${k} 0 0 1 ${k} ${k} L ${w - c} ${k} A ${c} ${c} 0 0 1 ${w} ${k + c} L ${w} ${h - k - c} A ${c} ${c} 0 0 1 ${w - c} ${h - k} L ${k} ${h - k} A ${k} ${k} 0 0 1 0 ${h} Z`;
}

// MARK: 收起态

/**
 * 「这条快照要不要显示」——**全文件唯一的一处判据**。
 *
 * 此前同一规则散成两种拼法：`s.process_running` 与
 * `snap.process_running || snap.level !== 'offline'`，分布在 5 个调用点。
 * 今天它们**可证明等价**（Rust `decide_level` 在 `!process_running` 时
 * 无条件返回 `Offline`，engine.rs:435-441，所以 `level !== 'offline'` 蕴含
 * `process_running`），所以第二种是冗余而非修正。
 *
 * 冗余不是保险，是**伪装成有意为之的隐患**：哪天 `decide_level` 加一个
 * 「进程没跑但仍给出非 Offline」的分支，这两处就会显示出别处藏着的条目，
 * 而且没有任何测试会红。收在一处之后，改规则只需要改这里。
 *
 * 两侧守护：`registry` 侧验 `decide_level` 的契约（`!running ⇒ Offline`），
 * `ui_symbol_sentinel` 侧验本文件里**所有**过滤点都走这个函数。
 */
export const isVisible = (snap) => snap.process_running === true;

export function sliverSize(edge) {
  return edge === 'left' || edge === 'right'
    ? { w: 17, h: 88 }
    : { w: 88, h: 17 };
}

export function renderSliver() {
  const st = getState();
  const root = document.getElementById('root');
  const eng = st.engine;
  const working = !!eng?.any_working;
  const alert = !!getTaskAttention()?.total || !!getTaskAttention()?.error || !!eng?.has_attention || eng?.latest_event?.event_type === 'costSpike';

  const edge = st.settings?.dock_edge ?? 'top';
  const hitClass = edge === 'top' ? 'hit-top' : edge === 'bottom' ? 'hit-bottom'
    : edge === 'left' ? 'hit-left' : 'hit-right';
  const vertical = edge === 'left' || edge === 'right';

  // 「收起后隐藏贴条」：不画那条可见的细条，但**保留热区**。
  //
  // 为什么保留：微细条是收起态唯一的唤回入口（`mouseenter` 即展开）。
  // 连热区一起去掉的话，窗口会变成一块看不见也点不到的死区——
  // 用户只能靠托盘或快捷键把它叫出来，而那两样在 macOS 侧还要用户自己知道。
  // 所以「隐藏」隐藏的是**胶囊把手**，不是交互面积。
  const hidden = st.settings?.hide_docked_sliver === true;
  const status = alert ? '有提醒' : !eng ? '等待采样' : working ? '工作中' : '待机';
  const className = `sliver ${vertical ? 'vertical' : ''} ${hitClass} ${working ? 'working' : ''} ${alert ? 'alert' : ''}${hidden ? ' sliver-invisible' : ''}`;
  const existing = root.querySelector('#sliver');
  // 周期采样只更新状态，保留动画相位与键盘焦点。
  if (existing) {
    existing.className = className;
    existing.setAttribute('aria-label', `展开灵动岛：${status}`);
    existing.title = `${status} · 展开灵动岛`;
    return;
  }
  root.innerHTML = `
    <div class="${className}" id="sliver" role="button" tabindex="0" aria-label="展开灵动岛：${status}" title="${status} · 展开灵动岛">
      <div class="sliver-capsule" aria-hidden="true"><span class="sliver-state"></span></div>
    </div>`;

  const el = root.querySelector('#sliver');
  el.addEventListener('mouseenter', () => expand());
  el.addEventListener('mouseup', () => expand());
  el.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); expand(); }
  });
}

// MARK: 顶栏状态摘要（HeaderPresentation）

function headerPresentation(eng) {
  const visible = eng?.snapshots.filter(isVisible) ?? [];
  if (eng?.latest_event && ['attention', 'costSpike'].includes(eng.latest_event.event_type)) {
    const ev = eng.latest_event;
    const isCost = ev.event_type === 'costSpike';
    // 还有几条在排队：不显示它就等于把队列藏起来——用户会以为「关掉这条就没事了」。
    // 计数来自 Rust 的 `pending_events`（不含正在显示的这条）。
    const pending = eng.pending_events ?? 0;
    const resource = !ev.externally_delivered && /(?:^|\s)kind=(?:memory|hung)\b/.test(ev.detail ?? '');
    const base = ev.externally_delivered ? (isCost ? '外部告警' : '外部确认') : isCost ? '用量提醒' : resource ? '资源提醒' : '待确认';
    return {
      title: ev.agent_name,
      subtitle: eventSummary(ev),
      badge: pending > 0 ? `${base} +${pending}` : base,
      tint: isCost ? 'var(--danger)' : 'var(--warning)',
      iconChar: isCost ? '\uE7BA' : '\uE7C2',
    };
  }
  const waiting = visible.find((s) => s.level === 'attention');
  if (waiting) {
    return {
      title: waiting.name,
      subtitle: waiting.current_action ?? '等待你确认',
      badge: '待确认',
      tint: 'var(--warning)',
      iconChar: '\uE7C2',
    };
  }
  const workingCount = visible.filter((s) => s.level === 'working').length;
  const completedCount = visible.filter((s) => s.level === 'completed').length;
  const uncertainCount = visible.filter((s) => s.level === 'idle' && s.observability?.code && s.observability.code !== 'observed').length;
  const badge = workingCount > 0 ? `${workingCount} 工作中`
    : completedCount > 0 ? `${completedCount} 已完成`
      : uncertainCount > 0 ? `${uncertainCount} 待核实`
        : visible.length ? '待机' : null;
  return { title: '智能体', badge, tint: workingCount > 0 ? 'var(--working)' : uncertainCount > 0 ? 'var(--warning)' : 'var(--idle)' };
}

// MARK: Agent 行（AgentRowView）

// 健康度徽标：分数与等级都来自快照的 `health`（Rust 侧 health.rs，对齐 Swift
// AgentHealthEvaluator），界面**不自己判定**任何一档——包括「健康就不显示」这条
// 也只是显示策略。等级为 healthy 时不占位：这一行是常显列表，
// 默认状态挂一枚「健康」是纯噪音（Swift 那边是点开详情才看到，等价）。
function healthChip(snap) {
  const health = snap.health;
  if (!health || health.grade === 'healthy') return '';
  const tips = [health.summary, ...(health.issues ?? []), health.suggestion].filter(Boolean).join(' · ');
  const label = healthReason(snap)?.label ?? (health.grade === 'partial' ? '观测不全' : '资源提醒');
  return `<span class="health-chip" data-grade="${esc(health.grade)}" title="${esc(`运行健康度 ${health.score}/100 · ${tips}`)}">${label}</span>`;
}

/// 「可观测性判定」在界面上的说法。**只有这一份**：
/// 卡片与侧边栏若各写一份，改一处就会出现「同一个状态两种说法」。
const UNCERTAIN_LABELS = {
  blindSessionSource: '会话源读不到',
  noLocalData: '无本地明细',
  sourceNotWired: '未接入明细源',
};

/// 一行 Agent 的**显示内容**（灵动岛与侧边栏共用）。
///
/// 为什么只共用「内容」而不共用「排版」：字段/文案/颜色的分叉**没人拦**
/// （`models.rs` 那条哨兵只拦「字段不存在」），而排版的分叉是**有意的**——
/// 灵动岛的行有进度环、动作条与迷你趋势点，侧边栏的行只是两行字。
/// 把内容抽出来之后，两种形态各自排版，但「显示什么、怎么措辞、什么颜色」只有一处。
export function agentRowModel(snap) {
  const colors = levelColors(snap.level);
  const uncertainty = snap.level === 'idle' ? UNCERTAIN_LABELS[snap.observability?.code] : null;
  const hasAction = ['working', 'attention'].includes(snap.level);
  return {
    id: snap.id,
    name: snap.name,
    level: snap.level,
    /** 状态那一格显示什么：判不出时**不能**说「空闲」——那会把「读不到」说成「闲着」。
     *  出处后缀（` · 自报` / ` · 自报冲突`）由 Rust 拼好，界面只做拼接——
     *  「观测/推断是常态，不挂标签」这条规则只在 Rust 侧有一份。 */
    statusText: (uncertainty ?? snap.level_label) + (snap.provenance_suffix ?? ''),
    statusColor: uncertainty ? 'var(--warning)' : colors.fg,
    statusBackground: uncertainty ? 'color-mix(in srgb, var(--warning) 10%, transparent)' : colors.bg,
    statusBorder: uncertainty ? 'color-mix(in srgb, var(--warning) 25%, transparent)' : colors.border,
    /** 24h 用量：没取到写 `—`（不是 0） */
    tokensText: snap.token_usage != null ? compact(snap.token_usage.tokens24h) : '—',
    /** 只有工作/等待确认才有「当前动作」，其余形态是空的 */
    actionText: hasAction ? (snap.current_action ?? '') : '',
    hasAction: hasAction && !!snap.current_action,
    activityText: snap.last_activity_text ?? '',
    isAttention: snap.level === 'attention',
  };
}

function rowHtml(snap) {
  const model = agentRowModel(snap);
  const target = getState().engine?.session_navigation?.[snap.id];
  const next = taskCoversSnapshot(getTaskAttention(), snap, target) ? null : agentNextStep(snap, target);
  const followup = next ? `<div class="agent-next-step"><span title="${esc(`${next.kind}：${next.detail}`)}">${esc(next.detail)}</span>${next.action ? `<button type="button" data-open-agent="${esc(snap.id)}" data-expected-url="${esc(target?.url ?? '')}" aria-label="${esc(`${snap.name}：${next.action}`)}" title="${esc(next.hint)}">${esc(next.action)}${ICONS.chevRight}</button>` : ''}</div>` : '';
  return `
    <div class="agent-entry"><button type="button" class="row" data-agent="${esc(model.id)}" aria-label="${esc(model.name)}：${esc(model.statusText)}，查看详情">
      <div class="row-line1">
        ${agentIcon(snap, 'island-agent-icon')}
        <div class="row-name-col">
          <div class="row-name">${esc(model.name)}</div>
          <div class="row-sub"><i class="island-state-dot" style="background:${model.statusColor}"></i><span>${esc(model.statusText)}</span></div>
        </div>
        <div class="island-row-usage"><strong>${model.tokensText}</strong><span>24h tokens</span></div>
        ${healthChip(snap)}
      </div>
      ${model.hasAction && snap.level !== 'attention' ? `<div class="action-bar" title="${esc(model.actionText)}">${esc(model.actionText)}</div>` : ''}
    </button>${followup}</div>`;
}

// MARK: 主卡（IslandView.expandedCard）

export function renderCard() {
  const st = getState();
  const eng = st.engine ?? {
    snapshots: [], grand_total: {},
    latest_event: null, any_working: false, has_attention: false,
    dock_edge: st.settings?.dock_edge ?? 'top',
  };
  const root = document.getElementById('root');
  const edge = st.settings?.dock_edge ?? 'top';
  const visible = eng.snapshots.filter(isVisible);
  const dark = document.documentElement.classList.contains('theme-dark');

  let content = '';
  if (st.route === 'tokenAnalytics') {
    content = pageAnalytics(eng);
  } else if (st.route === 'settings') {
    content = `<div class="page island-settings" data-island-settings-page><div class="page-header"><button type="button" class="icon-btn" data-back aria-label="返回监控">${ICONS.chevLeft}</button><span class="page-title">设置</span><span class="settings-save-status">自动保存</span></div><div class="island-settings-body">${pageSettings()}</div></div>`;
  } else if (st.route.startsWith('agentDetail:')) {
    content = pageAgentDetail(eng, st.route.split(':')[1]);
  } else {
    content = listCard(eng, st, visible, dark, edge);
  }

  const existing = root.querySelector('.card');
  if (isIsland() && root.dataset.motionRoute === st.route && navigationActive(existing)) return;
  const previousRoute = root.dataset.motionRoute;
  if (previousRoute != null && previousRoute !== st.route) clearTimeout(st.collapseTimer);
  const focused = document.activeElement;
  const focusLabel = root.dataset.motionRoute === st.route && root.contains?.(focused) ? focused?.getAttribute('aria-label') : null;
  let captured = isIsland() ? captureNavigation(existing, root.dataset.motionRoute, st.route, st.route === 'list') : null;
  if (!isIsland()) rememberPage(root, st.route, '.card-inner');
  const opening = !existing;
  const markup = `<div class="card-inner${opening ? ' card-enter' : ''}">${content}</div>`;
  if (isIsland() && existing) {
    existing.className = `card dock-${edge}`;
    existing.innerHTML = markup;
  } else root.innerHTML = `<div class="card dock-${edge}">${markup}</div>`;
  const changed = root.dataset.motionRoute != null && root.dataset.motionRoute !== st.route;
  if (isIsland()) root.dataset.motionRoute = st.route;
  else pageMotion(root, st.route, '.card-inner');
  bindCardEvents(eng, st);
  bindTaskAttention(root);
  const card = root.querySelector('.card');
  if (captured?.layoutOnly) {
    // A newly sampled action can add/remove a row just after returning. Give
    // that height change the same continuous surface instead of a second jump.
    card.style.maxHeight = 'none';
    const next = card.getBoundingClientRect();
    card.style.maxHeight = '';
    if (Math.abs(next.height - captured.from.height) <= 1 && Math.abs(next.width - captured.from.width) <= 1) {
      captured.copy.remove(); captured = null;
    }
  }
  if (captured && card) {
    stageNavigation(card, captured);
    if (captured.layoutOnly && focusLabel) [...root.querySelectorAll('[aria-label]')].find(n => n.getAttribute('aria-label') === focusLabel)?.focus({ preventScroll: true });
  }
  else if (isIsland() && changed && card) focusNavigation(card, st.route, previousRoute);
  else if (focusLabel) [...root.querySelectorAll('[aria-label]')].find(n => n.getAttribute('aria-label') === focusLabel)?.focus({ preventScroll: true });
  if (changed && card) resizeToContent();
}

function listCard(eng, st, visible, dark, edge) {
  const hp = headerPresentation(eng);
  const gt = eng.grand_total ?? {};
  const ev = eng.latest_event;
  const canOpenEvent = !!(eng.event_navigation?.[ev?.id] ?? eng.session_navigation?.[ev?.agent_id]);

  const chev = islandToolbarIcon(edge);

  const banner = ev && !taskCoversEvent(getTaskAttention(), eng) ? `
    <div class="banner" style="background:color-mix(in srgb, ${ev.event_type === 'costSpike' ? 'var(--danger)' : 'var(--warning)'} 10%, transparent)">
      <div class="line1">
        <button type="button" class="banner-message" data-banner-detail aria-expanded="${!!st.lastEventDetail}" aria-controls="island-event-detail" title="${esc(eventSummary(ev))}" aria-label="${esc(bannerTitle(ev))}，${st.lastEventDetail ? '收起' : '展开'}完整提醒">${esc(bannerTitle(ev))}</button>
        ${canOpenEvent ? `<button type="button" class="banner-control" data-agent-jump="${esc(ev.agent_id)}" aria-label="打开 ${esc(ev.agent_name)} 工具" title="打开工具">${ICONS.chevRight}</button>` : ''}
        <button type="button" class="banner-control" data-banner-close aria-label="忽略这条提醒" title="忽略提醒">${ICONS.close}</button>
      </div>
      <div class="detail" id="island-event-detail"${st.lastEventDetail ? '' : ' hidden'}>${esc(eventSummary(ev))}${ev.detail ? `<p>${esc(ev.detail)}</p>` : ''}</div>
    </div>` : '';

  const search = st.searchActive ? `
    <div class="searchbar">
      ${ICONS.search.replace('<svg', '<svg width="10" height="10" style="color:var(--cyan)"')}
      <input id="searchInput" placeholder="搜索名称或 CLI…" value="${esc(st.searchText ?? '')}" />
    </div>` : '';

  const filtered = st.searchActive && st.searchText
    ? visible.filter((s) => s.name.toLowerCase().includes(st.searchText.toLowerCase()) || s.id.includes(st.searchText.toLowerCase()))
    : visible;

  const list = filtered.length === 0
    ? `<div class="empty"${!st.engine ? ' role="status"' : ''}>${!st.engine ? `${navigationIcon('terminal')}<span>等待采样</span>` : st.searchActive ? `${navigationIcon('search')}<span>未找到匹配「${esc(st.searchText)}」的智能体</span>`
      : `${navigationIcon('terminal')}<span>暂无在线智能体</span>`}</div>`
    : `<div class="list">${filtered.map(rowHtml).join('')}</div>`;

  const summary = `
    <button type="button" class="summary" data-analytics aria-label="查看用量分析" title="Token 用量 · 查看分析">
      <span class="island-summary-metric"><span>24h</span><strong>${gt.tokens24h == null ? '—' : compact(gt.tokens24h)}</strong></span>
      <span class="island-summary-metric"><span>累计</span><strong>${gt.tokens_total == null ? '—' : compact(gt.tokens_total)}</strong></span>
      <span class="island-summary-link" aria-hidden="true">${ICONS.chevRight}</span>
    </button>`;

  const statusColor = hp.tint;

  return `
    <div class="header" data-drag>
      <div class="status-dot" style="background:${statusColor}"></div>
      <div class="header-titles">
        <div class="header-line1">
          <span class="header-title" title="${esc(hp.title)}">${esc(hp.title)}</span>
          ${eng.demo ? '<span class="badge" style="color:var(--cyan);background:color-mix(in srgb, var(--cyan) 14%, transparent);border:0.5px solid color-mix(in srgb, var(--cyan) 35%, transparent)">演示数据</span>' : ''}
          ${hp.badge ? `<span class="badge" style="color:${hp.tint};background:color-mix(in srgb, ${hp.tint} 14%, transparent);border:0.5px solid color-mix(in srgb, ${hp.tint} 35%, transparent)">${esc(hp.badge)}</span>` : ''}
        </div>
        <span class="header-count">${st.engine ? `${visible.length} 在线` : '等待采样'}</span>
      </div>
      <div class="header-icons">
        <button type="button" class="icon-btn" data-search title="搜索（/）" aria-label="搜索智能体">${islandToolbarIcon('search')}</button>
        <button type="button" class="icon-btn" data-island-settings title="设置" aria-label="设置">${islandToolbarIcon('settings')}</button>
        <button type="button" class="icon-btn" data-collapse title="收起灵动岛" aria-label="收起灵动岛">${chev}</button>
      </div>
    </div>
    <div class="divider"></div>
    ${banner}
    <div data-task-attention-slot>${taskAttentionHtml(getTaskAttention(), eng)}</div>
    ${search}
    ${list}
    ${summary}`;
}

// MARK: Token 分析页（TokenAnalyticsView）

export function pageAnalytics(eng) {
  return `
    <div class="page" data-page="tokenAnalytics">
      <div class="page-header">
        <button type="button" class="back-btn" aria-label="返回监控" data-back>‹</button>
        <div class="page-titles">
          <div class="t">Token 用量</div>
          <div class="s">净消耗 · 不含缓存读取</div>
        </div>
      </div>
      <div class="divider"></div>
      <div class="page-body" data-report-root>
        <div class="c-faint" style="font-size:11px;text-align:center;padding:20px 0">加载中…</div>
      </div>
    </div>`;
}

function renderReportBody(report, showContext = false) {
  const u = report.usage;
  const now = new Date();
  const remaining = Math.max(1, new Date(now.getFullYear(), now.getMonth() + 1, 0).getDate() - now.getDate());
  const hourly = report.hourly30d;

  const records=hourRecords(hourly),peak24=Math.max(1,...records.map(record=>record.tokens??0));
  const heat=records.map(record=>{const label=`${new Date(record.ts).toLocaleString()} · ${record.tokens===null?'无小时记录':`${record.tokens} tokens`}`;const alpha=record.tokens===null?0:record.tokens===0?0.08:Math.max(0.15,Math.sqrt(record.tokens/peak24)*0.9);return `<i role="img" aria-label="${esc(label)}" title="${esc(label)}"${record.tokens===null?' class="is-missing"':''} style="background:color-mix(in srgb, var(--cyan) ${Math.round(alpha*100)}%, transparent)"></i>`;});

  const maxTool = Math.max(1, ...report.models24h.map((m) => m.tokens));

  return `
    <div class="card-box usage-forecast">
      <h4>月底前预估</h4>
      <div class="totals" style="margin-top:8px;justify-content:flex-start;gap:14px">
        <div><div class="big-num" style="font-size:14px">${compact(u.tokens24h * remaining)}</div><div class="num-label">剩余用量</div></div>
        <div><div class="big-num c-working" style="font-size:14px">${costText(u.cost24h, u.cost_estimated) ? `~$${(u.cost24h * remaining).toFixed(2)}` : '—'}</div><div class="num-label">剩余费用</div></div>
        <div><div class="big-num" style="font-size:14px">${remaining} 天</div><div class="num-label">剩余天数</div></div>
      </div>
    </div>
    <div class="card-box usage-totals">
      <div class="totals">
        <div><div class="big-num">${compact(u.tokens24h)}</div><div class="num-label">24h 用量</div></div>
        <div><div class="big-num">${costText(u.cost24h, u.cost_estimated) || '—'}</div><div class="num-label">费用</div></div>
        <div><div class="big-num">${compact(u.tokens_total)}</div><div class="num-label">累计</div></div>
      </div>
    </div>
    ${trendCardHtml()}
    <div class="card-box usage-rhythm">
      <div style="display:flex;align-items:center"><span style="font-size:10.5px;color:var(--text-faint)">24 个小时桶</span>
        <span style="margin-left:auto;font-size:9.5px;color:var(--text)">有记录 ${records.filter(record=>record.tokens!==null).length}/24</span></div>
      <div class="heat">${heat.join('')}</div>
    </div>
    <div class="card-box usage-models">
      <h4>按模型用量 · 24h</h4>
      ${report.models24h.map((m) => `
        <div class="model-row">
          <div class="top"><span>${esc(m.model)}</span>
            <span class="r"><span class="tk">${compact(m.tokens)}</span><span class="cost">${costText(m.cost, m.cost_estimated)}</span></span></div>
          <div class="hbar"><i style="width:${Math.max(2, 100 * m.tokens / maxTool)}%"></i></div>
        </div>`).join('') || '<div class="c-faint" style="font-size:10px;margin-top:6px">暂无模型明细</div>'}
    </div>${showContext ? usageContextHtml(report.context24h, u.tokens24h, compact) : ''}`;
}

// MARK: Agent 详情页（AgentDetailView）

export function pageAgentDetail(eng, agentId) {
  const snap = eng.snapshots.find((s) => s.id === agentId);
  const name = snap?.name ?? agentId;
  return `
    <div class="page" data-page="agentDetail" data-agent-id="${esc(agentId)}">
      <div class="page-header">
        <button type="button" class="back-btn" aria-label="返回监控" data-back>‹</button>
        ${agentIcon(snap ?? { id: agentId, name })}
        <div class="page-titles">
          <div class="t">${esc(name)}</div>
          <div class="s">用量与模型</div>
        </div>
      </div>
      <div class="divider"></div>
      <div class="page-body" data-report-root>
        <div class="c-faint" style="font-size:11px;text-align:center;padding:20px 0">加载中…</div>
      </div>
    </div>`;
}

function renderDetailBody(report, snap) {
  const u = report.usage;
  const c = snap ? levelColors(snap.level) : levelColors('offline');
  return `
    <div style="display:flex;align-items:center;gap:8px">
      ${snap ? ringHtml(snap, 40) : ''}
      <div>
        <div style="font-size:12.5px;font-weight:600">${esc(snap?.name ?? '')}</div>
        <div style="font-size:9.5px;color:var(--text-faint)">${snap ? `${snap.level_label} · PID ${snap.pid ?? '—'} · 内存 ${snap.memory_text}` : '离线'}</div>
      </div>
    </div>
    <div class="card-box">
      <div class="totals" style="gap:16px">
        <div><div class="big-num" style="font-size:14px">${compact(u.tokens24h)}</div><div class="num-label">24h 用量</div></div>
        <div><div class="big-num" style="font-size:14px">${costText(u.cost24h, u.cost_estimated) || '—'}</div><div class="num-label">24h 费用</div></div>
        <div><div class="big-num" style="font-size:14px">${compact(u.tokens_total)}</div><div class="num-label">累计</div></div>
        <div><div class="big-num" style="font-size:14px">${costText(u.cost_total, u.cost_estimated) || '—'}</div><div class="num-label">累计费用</div></div>
      </div>
    </div>
    <div class="card-box">
      <h4>模型用量 · 24h</h4>
      ${report.models24h.map((m) => {
    const maxT = Math.max(1, ...report.models24h.map((x) => x.tokens));
    return `<div class="model-row" data-model="${esc(m.model)}">
          <div class="top"><span>${esc(m.model)}</span>
            <span class="r"><span class="tk">${compact(m.tokens)}</span><span class="cost">${costText(m.cost, m.cost_estimated)}</span></span></div>
          <div class="hbar"><i style="width:${Math.max(2, 100 * m.tokens / maxT)}%"></i></div>
        </div>`;
  }).join('') || `
        <div class="c-faint" style="font-size:11px;margin-top:6px">未发现本地明细</div>
        <div class="c-faint" style="font-size:9.5px;margin-top:4px">暂无可读取的用量记录。— 表示尚未获取数据。</div>`}
    </div>`;
}

// MARK: 卡片事件绑定

function bindCardEvents(eng, st) {
  const root = document.getElementById('root');

  // 顶栏拖拽 + 松手吸附
  const header = root.querySelector('[data-drag]');
  if (header) {
    header.setAttribute('data-tauri-drag-region', '');
    let dragStart = null;
    const interactive = (target) => target?.closest('button, input, textarea, select, a, [role="button"]');
    header.addEventListener('mousedown', (event) => {
      dragStart = event.button === 0 && !interactive(event.target)
        ? { x: event.screenX, y: event.screenY } : null;
    });
    header.addEventListener('mouseup', (event) => {
      const start = dragStart;
      dragStart = null;
      // 点击按钮或标题不代表拖拽；只在实际移动后吸附。
      if (!start || event.button !== 0 || interactive(event.target)
        || Math.hypot(event.screenX - start.x, event.screenY - start.y) < 5) return;
      setTimeout(async () => {
        const card = root.querySelector('.card');
        if (!st.expanded || !root.contains(header) || !card) return;
        try {
          const height = Math.min(card.getBoundingClientRect().height, 520);
          const edge = await invoke('snap_nearest_edge', { width: 330, height });
          st.settings.dock_edge = edge;
          applyEdge();
          renderCard();
        } catch (error) {
          invoke('log_from_ui', { message: `拖拽吸附失败：${error?.message ?? error}` }).catch(() => {});
        }
      }, 60);
    });
  }

  root.querySelectorAll('[data-back]').forEach((el) => el.addEventListener('click', () => {
    if (st.route.startsWith('agentDetail:')) { st.route = 'list'; }
    else { st.route = 'list'; }
    renderCard();
    resizeToContent();
  }));

  const searchBtn = root.querySelector('[data-search]');
  if (searchBtn) searchBtn.addEventListener('click', () => {
    st.searchActive = !st.searchActive;
    if (!st.searchActive) st.searchText = '';
    renderCard();
  });

  root.querySelectorAll('[data-analytics]').forEach((el) => el.addEventListener('click', () => {
    st.route = 'tokenAnalytics';
    renderCard();
    resizeToContent();
  }));

  root.querySelector('[data-island-settings]')?.addEventListener('click', () => {
    st.route = 'settings';
    renderCard();
    resizeToContent();
  });
  if (st.route === 'settings') bindSettings();

  const collapseBtn = root.querySelector('[data-collapse]');
  if (collapseBtn) collapseBtn.addEventListener('click', collapse);

  const jump = root.querySelector('[data-agent-jump]');
  if (jump) {
    const target = eng.event_navigation?.[eng.latest_event?.id];
    jump.title = target?.hint ?? '打开工具；此提醒暂不支持定位会话';
    jump.setAttribute('aria-label', target?.exactSession ? `打开 ${eng.latest_event.agent_name} 对应会话` : `打开 ${eng.latest_event?.agent_name ?? '智能体'} 工具`);
    jump.addEventListener('click', e => { e.stopPropagation(); openAgentSession(jump, jump.dataset.agentJump, eng.latest_event?.id, target?.url); });
  }

  const bannerDetail = root.querySelector('[data-banner-detail]');
  if (bannerDetail) bannerDetail.addEventListener('click', (e) => {
    e.stopPropagation();
    st.lastEventDetail = !st.lastEventDetail;
    renderCard();
    resizeToContent();
    root.querySelector('[data-banner-detail]')?.focus();
  });

  const bannerClose = root.querySelector('[data-banner-close]');
  if (bannerClose) bannerClose.addEventListener('click', async (e) => {
    e.stopPropagation();
    bannerClose.closest('.banner').style.display = 'none';
    resizeToContent();
    await invoke('clear_latest_event').catch(() => {});
  });

  const searchInput = root.querySelector('#searchInput');
  if (searchInput) {
    searchInput.addEventListener('input', () => {
      st.searchText = searchInput.value;
      // 仅刷新列表区
      const listZone = root.querySelector('.list');
      if (listZone) {
        const visible = (st.engine?.snapshots ?? []).filter(isVisible);
        const filtered = visible.filter((s) => s.name.toLowerCase().includes(st.searchText.toLowerCase()) || s.id.includes(st.searchText.toLowerCase()));
        listZone.innerHTML = filtered.length === 0
          ? `<div class="empty" style="padding:24px 0">${navigationIcon('search')}<span>未找到匹配「${esc(st.searchText)}」的智能体</span></div>`
          : filtered.map(rowHtml).join('');
        bindRowClicks(st);
      }
    });
    searchInput.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') { st.searchActive = false; st.searchText = ''; renderCard(); }
    });
    setTimeout(() => searchInput.focus(), 50);
  }

  bindRowClicks(st);

  root.onmouseleave = () => { if (st.expanded && !navigationActive(root.querySelector('.card'))) armCollapseTimer(); };
  root.onmouseenter = () => clearTimeout(st.collapseTimer);

  // 自愈高度：横幅/字体/异步内容造成的滞后由周期校正兜底
  if (!globalThis.__heightHealer) {
    globalThis.__heightHealer = setInterval(() => {
      const st2 = getState();
      if (st2.expanded && st2.windowVisible !== false) resizeToContent();
    }, 800);
  }

  // 分析页/详情页数据加载
  if (st.route === 'tokenAnalytics' || st.route.startsWith('agentDetail:')) hydrateReport();
}

async function openAgentSession(button, agent, event, expectedUrl) {
  if (button.disabled) return;
  clearTimeout(getState().collapseTimer);
  button.disabled = true;
  try {
    await invoke('open_agent_session', { agent, event: event ?? null, expectedUrl: expectedUrl || null });
  } catch (error) {
    const hint = document.createElement('div');
    hint.className = 'agent-open-error'; hint.setAttribute('role', 'alert');
    hint.textContent = String(error?.message ?? error);
    button.closest('.agent-entry, .wb-agent-entry, .banner')?.querySelector('.agent-open-error')?.remove();
    button.closest('.agent-entry, .wb-agent-entry, .banner')?.appendChild(hint);
    resizeToContent();
  } finally { if (button.isConnected) button.disabled = false; }
}

function bindRowClicks(st) {
  document.querySelectorAll('[data-open-agent]').forEach(el => {
    el.onclick = event => { event.stopPropagation(); openAgentSession(el, el.dataset.openAgent, null, el.dataset.expectedUrl); };
  });
  document.querySelectorAll('[data-agent]').forEach((el) => {
    el.onclick = () => {
      st.route = `agentDetail:${el.dataset.agent}`;
      renderCard();
    };
  });
}

export async function hydrateReport() {
  const st = getState();
  const bodies = [...document.querySelectorAll('[data-report-root]')].filter(body => !body.closest('[data-page-outgoing]'));
  await Promise.all(bodies.map(async (body) => {
    const page = body.closest('[data-page]');
    const analytics = page?.dataset.page === 'tokenAnalytics';
    // 空 ID 是后端约定的全部启用档案汇总，不取第一个在线工具。
    const agentId = analytics ? '' : page?.dataset.agentId ?? '';
    const current=pageRequest(body);
    body.setAttribute('aria-busy','true');
    let report;
    try{report=await invoke('get_report',{agentId});}
    catch{
      if(!current())return;
      body.setAttribute('aria-busy','false');
      if(body.dataset.reportReady!=='true'&&!body.querySelector('[data-usage-read-error]'))body.replaceChildren();
      if(!body.querySelector('[data-usage-read-error]'))body.insertAdjacentHTML('afterbegin',`<div class="usage-read-error" data-usage-read-error><p role="status">用量读取失败${body.dataset.reportReady==='true'?'，保留上次内容':''}。请重试。</p><button type="button" class="mini-btn" data-usage-retry>重试</button></div>`);
      body.querySelector('[data-usage-retry]').onclick=()=>hydrateReport();return;
    }
    // 导航可能已经换页；过期响应不写入新页面。
    if (!current()) return;
    body.setAttribute('aria-busy','false');
    body.dataset.reportReady = 'true';
    if (!report) {
      body.innerHTML = `<div class="report-empty">${navigationIcon('chart')}<span>暂无用量明细</span></div>`;
      return;
    }
    const snap = st.engine?.snapshots.find((entry) => entry.id === agentId);
    const focus=document.activeElement;const rangeFocused=body.contains(focus)&&(focus.hasAttribute('data-usage-range')||focus.hasAttribute('data-usage-retry'));
    const contextFocused=body.contains(focus)&&focus.matches('[data-usage-context]>summary');
    const contextDetail=body.querySelector('[data-usage-context]');
    if(contextDetail)body.dataset.usageContextOpen=String(contextDetail.open);
    body.innerHTML = analytics ? renderReportBody(report, isWorkbench()&&st.workbenchPage==='tokenAnalytics') : renderDetailBody(report, snap);
    bindUsageContext(body);
    if(contextFocused)body.querySelector('[data-usage-context]>summary')?.focus({preventScroll:true});
    if(analytics){bindTrend(body.querySelector('[data-usage-trend]'),report.hourly30d,body.dataset.trendRange??'24',range=>{body.dataset.trendRange=range;});if(rangeFocused)body.querySelector(`[data-usage-range="${body.dataset.trendRange}"]`)?.focus({preventScroll:true});}
  }));
  if (isIsland()) await resizeToContent();
}

// MARK: - 侧边栏形态

/// 侧边栏壳：左导航 + 右内容。
///
/// 与灵动岛的**数据与页面函数完全共用**，只是容器不同——这就是「监控模块不依赖容器尺寸」
/// 这句要求的最小落地：`pageAnalytics` / `pageAgentDetail` 原样复用，
/// 侧边栏自己只负责导航与列表。
// MARK: Provider 页（Codex 档位切换）
//
// **这一页原先只有注水函数、没有页面本身**：导航项在、`hydrateProvider` 在、
// `renderProviderPage` 在，可是渲染页面的 `pageProvider()` 从没被定义过——
// 于是点「Codex 档位」直接抛 `ReferenceError`。
// 一条只有产生点、没有声明的链路会让人以为「页面做完了」，
// 而实际点进去就是白屏——静态检查与冒烟都发现不了，它们只看「有没有报错」。
//
// 所以这一页只做一件事：**给出容器**。内容全部由 `hydrateProvider` →
// `renderProviderPage` 填，措辞与字段名都在那边一处（哨兵盯着字段名）。
//
// ⚠️ **这个函数头在 v0.0.200 那次提交里被误删过，潜伏到 v0.0.229 才找回来。**
// 后果不是「档位页少一块」，而是**整个模块解析失败**：函数体剩下一个顶层
// `return`，而顶层 `return` 在 ES 模块里是语法错误 ⇒ `views.js` 整个加载不了
// ⇒ 灵动岛 / 侧边栏 / 工作台**三个形态全是空白**。连续 29 个版本。
// 它能潜伏这么久，是因为一直用 `node --check app/ui/js/views.js` 验语法——
// **那是按「脚本」解析的**，而模块是严格模式，两者判定不同（脚本模式不报
// 「顶层 return」）。正确做法是按 `.mjs` 交给 `node --check` 或直接 `import()`；
// 那条检查现在是 `ui_symbol_sentinel::every_ui_js_file_parses_as_a_module`。
// MARK: 报告面板（工作台）
//
// 报告**只读不改**：`report` 命令已经能生成 md / csv，而报告是拿去对账的东西，
// 从界面上写盘会多出一条「写到哪去了」的路径。所以这里只生成 + 复制，
// 要落盘用 CLI——那条路已经验过（原子写、写失败 exit 1）。
export function pageReport() { return reportPanelHtml(); }

export async function hydrateReportPanel(format) {
  const panel=visibleReportPanel();if(panel&&format)await bindReport(panel).generate(format);
}

export function pageProvider() {
  if (isWorkbench) return `<div class="model-workspace" data-model-workspace>
    <div class="model-navigation" role="group" aria-label="模型与连接视图">
      <button type="button" class="mini-btn" data-model-view="tools" aria-pressed="true" aria-controls="model-tools">工具配置</button>
      <button type="button" class="mini-btn" data-model-view="models" aria-pressed="false" aria-controls="model-directory">模型目录</button>
      <button type="button" class="mini-btn" data-model-view="extensions" aria-pressed="false" aria-controls="model-extensions">扩展配置</button>
      <button type="button" class="mini-btn" data-model-view="prompts" aria-pressed="false" aria-controls="model-prompts">提示词</button>
      <button type="button" class="mini-btn" data-model-view="services" aria-pressed="false" aria-controls="model-services">服务连接</button>
    </div>
    <section id="model-tools" data-model-panel="tools" aria-label="Codex 工具配置"><h2 class="model-tool-title">Codex <span>本机配置</span></h2>
      <div class="sb-page" data-provider-root><div class="sb-empty">加载中…</div></div>
    </section>
    <section id="model-directory" data-model-panel="models" aria-label="本机配置模型目录" hidden><div data-model-directory><div class="sb-empty">加载中…</div></div></section>
    <section id="model-extensions" data-model-panel="extensions" aria-label="扩展与来源能力" hidden>${pageMcp()}${pageClaudePlan()}</section>
    <section id="model-prompts" data-model-panel="prompts" aria-label="提示词与用户指令" hidden>${pagePrompts()}</section>
    <section id="model-services" data-model-panel="services" aria-label="外部服务连接" hidden>${pageConnections()}</section>
  </div>`;
  return `
    <div class="sb-page" data-provider-root>
      <div class="sb-empty">加载中…</div>
    </div>`;
}

// MARK: 工作台形态（第三个窗口）
//
// 概览与独立功能页复用既有页面函数；容器不复制业务实现。

function workbenchStatus(engine) {
  if (!engine) return '等待采样';
  const running = engine.snapshots.filter(isVisible);
  const attention = running.filter((snap) => snap.level === 'attention').length;
  return attention ? `${attention} 个等待确认` : running.length ? `${running.length} 个在线` : '暂无在线智能体';
}

/** 工作台左栏：监控列表。与侧边栏同一份显示模型（`agentRowModel`）。 */
function workbenchMonitor(eng) {
  if (!eng) return `<div class="wb-empty" role="status">${navigationIcon('terminal')}<strong>等待采样</strong><span>正在读取本机运行状态。</span></div>`;
  const route = getState().route;
  if (route.startsWith('agentDetail:')) return pageAgentDetail(eng, route.slice('agentDetail:'.length));
  const running = eng.snapshots.filter(isVisible);
  if (running.length === 0) {
    return `<div class="wb-empty">${navigationIcon('terminal')}<strong>暂无在线智能体</strong><span>启动编码工具后即可查看状态。</span></div>`;
  }
  return running
    .map((snap) => {
      const model = agentRowModel(snap);
      const target = eng.session_navigation?.[snap.id];
      const next = agentNextStep(snap, target);
      const detail = [model.statusText, next ? null : model.actionText || model.activityText].filter(Boolean).join(' · ');
      return `<div class="wb-agent-entry"><button type="button" class="wb-agent" data-agent="${esc(model.id)}">
        ${agentIcon(snap, 'wb-agent-glyph')}
        <span class="name">${escapeHtml(model.name)}</span>
        <span class="tokens">${escapeHtml(model.tokensText)}</span>
        <span class="meta"><i class="wb-state-dot" style="background:${model.statusColor}"></i>${escapeHtml(detail)}</span>
      </button>${next ? `<div class="agent-next-step"><span title="${esc(next.kind)}：${esc(next.detail)}">${esc(next.detail)}</span>${next.action ? `<button type="button" data-open-agent="${esc(snap.id)}" data-expected-url="${esc(target?.url ?? '')}" aria-label="${esc(snap.name)}：${esc(next.action)}" title="${esc(next.hint)}">${esc(next.action)}${navigationIcon('forward')}</button>` : ''}</div>` : ''}</div>`;
    })
    .join('');
}

// 工作台导航独立于智能体详情路由；周期采样只更新实时监控区。
function navigationIcon(kind) {
  const paths = {
    square: '<rect x="3" y="3" width="18" height="18" rx="5"/><path d="M9 3v18M9 10h12"/>',
    chart: '<path d="M4 20h16M6 16v-5m6 5V5m6 11V8"/>',
    check: '<rect x="3" y="3" width="18" height="18" rx="5"/><path d="m7 12 3 3 7-7"/>',
    sliders: '<path d="M4 7h6m4 0h6M4 17h10m4 0h2"/><circle cx="12" cy="7" r="2"/><circle cx="16" cy="17" r="2"/>',
    document: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9ZM14 3v6h6M8 13h8M8 17h5"/>',
    terminal: '<rect x="3" y="4" width="18" height="16" rx="4"/><path d="m7 9 3 3-3 3m6 0h4"/>',
    bell: '<path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4"/>',
    forward: '<path d="m9 6 6 6-6 6"/>',
    gear: '<circle cx="12" cy="12" r="3"/><path d="m9 3-1 3-3 1-2 3 2 2-1 3 3 2 3-1 2 3 3-2v-3l3-1 1-3-3-2V7l-3-1-1-3Z"/>',
  };
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[kind] ?? paths.square}</svg>`;
}

const workbenchDescriptions = {
  tokenAnalytics: '用量、趋势与模型分布。净消耗不含缓存读取。',
  todo: '记录待办，勾选完成。',
  sessions: '查看可信来源，进入会话或关联任务。',
  tasks: '整理任务、运行记录与需处理事项。',
  workspaces:'组合项目、工具、配置与窗口布局。',
  windows: '预览排列所选工具窗口，并可撤销。',
  provider: '管理工具配置、模型目录与服务地址。',
  report: '导出本机用量报告。',
  agents: '选择要监控的智能体。',
  remote: '离开电脑时接收状态通知。',
  settings: '调整外观、采样与通知。',
};

function navigationSummary(engine) {
  if (!engine) return '<span class="nav-machine-label">本机状态</span><p>等待采样</p>';
  return `<span class="nav-machine-label">本机状态</span><div class="nav-machine-values"><div><strong>${engine.snapshots.filter(isVisible).length}</strong><span>在线</span></div><div><strong>${compact(engine.grand_total.tokens24h)}</strong><span>24h tokens</span></div></div><p>数据保存在本机</p>`;
}

const attentionMarkup = new WeakMap();
export function renderTaskAttentionOnly() {
  document.querySelectorAll('[data-task-attention-slot]').forEach(slot => {
    const html=taskAttentionHtml(getTaskAttention(),getState().engine);
    if (attentionMarkup.get(slot) === html) return;
    const focused=document.activeElement;
    const focusAction=slot.contains(focused) ? ['data-task-summary-detail','data-task-summary-source','data-task-summary-list'].find(key=>focused.hasAttribute(key)) : null;
    slot.innerHTML=html; attentionMarkup.set(slot,html);
    bindTaskAttention(slot);
    if (focusAction) slot.querySelector(`[${focusAction}]`)?.focus({preventScroll:true});
  });
}

export function renderNavSummaryOnly() {
  const box = document.querySelector('[data-nav-summary]');
  if (box) box.innerHTML = navigationSummary(getState().engine);
}

export function renderWorkbench() {
  const st = getState();
  const eng = st.engine;
  const showReport=st.workbenchPage==='report';
  const desired=showReport?'tokenAnalytics':st.workbenchPage;
  const selected = workbenchPages.some(([key]) => key === desired) ? desired : 'overview';
  st.workbenchPage = selected;
  const navigationPage = selected === 'todo' ? 'tasks' : selected;
  const title = workbenchPages.find(([key]) => key === navigationPage)?.[1] ?? '概览';
  const root = document.getElementById('root');
  const taskViewSwitch = navigationPage === 'tasks' && ['todo','tasks'].includes(root.dataset.motionRoute);
  if (root.querySelector('.wb-content') && root.dataset.motionRoute === selected) {
    workspaceFlow.route(selected);renderWorkspaceBanner();focusWorkspaceTarget();
    renderWorkbenchMonitorOnly();
    if(showReport)openUsageReport(root);
    return;
  }
  const section = (heading, content, attrs = '') => `<section class="wb-section"><h2 class="wb-section-title">${heading}</h2><div class="wb-section-body" ${attrs}>${content}</div></section>`;
  let content;
  if (selected === 'overview') {
    content = `<div class="wb-intro"><div><p class="wb-eyebrow">本机工作台</p><h1>工作概览</h1><p>查看运行状态，安排待办。</p></div><div class="wb-summary" data-wb-summary>${workbenchSummary(eng)}</div></div>
      <div class="wb-grid">
        <div class="wb-col wb-col-main">
          <div data-task-attention-slot>${taskAttentionHtml(getTaskAttention(), eng)}</div>
          ${section('实时监控', workbenchMonitor(eng), `data-wb-monitor data-detail-route="${esc(st.route)}"`)}
          ${section('待办事项', pageTodo())}
        </div>
        <div class="wb-col">
          <section class="wb-section wb-tools"><h2 class="wb-section-title">常用工具</h2><div class="wb-section-body">
          ${[['tokenAnalytics', '用量分析', 'chart', '查看趋势与模型消耗'], ['provider', '模型与连接', 'sliders', '管理配置、模型与备份'], ['report', '导出报告', 'document', '生成 Markdown 或 CSV']].map(([key, label, icon, detail]) => `<button type="button" class="wb-tool" data-wb-nav="${key}">${navigationIcon(icon)}<span><strong>${label}</strong><small>${detail}</small></span>${navigationIcon('forward')}</button>`).join('')}
          </div></section>
        </div>
      </div>`;
  } else {
    const pages = {
      tokenAnalytics: () => `${toolBudgetsHtml()}<details class="usage-report" data-usage-report><summary>用量报告<span>Markdown · CSV</span></summary>${pageReport()}</details>${pageAnalytics(eng)}`, provider: pageProvider, todo: pageTodo, tasks: pageTasks, sessions: pageSessions, windows: pageWindowLayout,workspaces:pageWorkspaces,
      report: () => pageReport(), settings: pageSettings, remote: pageRemote, agents: pageAgents,
    };
    const icon = workbenchPages.find(([key]) => key === navigationPage)?.[2];
    const taskNavigation = navigationPage === 'tasks' ? `<nav class="wb-task-navigation" aria-label="任务视图">${[['tasks','任务看板'],['todo','待办事项']].map(([key,label])=>`<button type="button" class="mini-btn" data-wb-task-view data-wb-nav="${key}" aria-current="${selected===key?'page':'false'}">${label}</button>`).join('')}</nav>` : '';
    const description = navigationPage === 'tasks' ? '整理待办、任务与运行记录。' : workbenchDescriptions[selected] ?? '';
    const panel = pages[selected]?.() ?? '';
    content = `<div class="wb-single" data-workbench-page="${selected}"><div class="wb-page-heading"><span class="wb-page-icon">${navigationIcon(icon)}</span><div><h1>${title}</h1><p>${description}</p></div></div>${taskNavigation}${navigationPage==='tasks'?`<div data-task-view-panel>${panel}</div>`:panel}</div>`;
  }
  const previousContent = root.querySelector('.wb-content');
  if (previousContent) workbenchScroll.set(root.dataset.motionRoute, previousContent.scrollTop);
  const motionSurface = taskViewSwitch ? '[data-task-view-panel]' : '.wb-content';
  rememberPage(root, selected, motionSurface);
  const restored = workbenchPagesCache.take(selected);
  if (root.querySelector('.wb')) {
    workbenchPagesCache.remember(root.dataset.motionRoute, previousContent);
    const nextContent = restored ?? document.createElement('div');
    nextContent.setAttribute('tabindex', '-1');
    nextContent.setAttribute('aria-label', '工作台内容');
    nextContent.className = `wb-content${selected === 'overview' ? ' wb-overview' : ''}`;
    if (!restored) nextContent.innerHTML = content;
    previousContent.replaceWith(nextContent);
    if(restored)nextContent.querySelector('[data-skills-packages]')?.dispatchEvent(new Event('skill-package-resume'));
    nextContent.scrollTop = workbenchScroll.get(selected) ?? 0;
    root.querySelector('.wb-head-title').textContent = title;
    root.querySelector('[data-wb-status]').textContent = workbenchStatus(st.engine);
    root.querySelectorAll('.wb-nav-item').forEach(button => {
      const active = button.dataset.wbNav === navigationPage;
      button.classList.toggle('is-active', active);
      button.setAttribute('aria-current', active ? 'page' : 'false');
    });
    renderNavSummaryOnly();
  } else root.innerHTML = `<div class="wb">
    <nav class="wb-nav" aria-label="工作台导航">
      <div class="wb-brand">${navigationIcon('square')}<span>AgentIsland<small>本机智能体工作台</small></span></div>
      <div class="wb-nav-links">
      <div class="wb-nav-label">工作</div>
      ${workbenchPages.filter(([key])=>key!=='todo').map(([key, label, icon]) => `${key === 'windows' ? '<div class="wb-nav-label wb-nav-divider">管理</div>' : ''}<button type="button" class="wb-nav-item${navigationPage === key ? ' is-active' : ''}" data-wb-nav="${key}" aria-current="${navigationPage === key ? 'page' : 'false'}">${navigationIcon(icon)}<span>${label}</span></button>`).join('')}
      </div>
      <div class="wb-nav-footer" data-nav-summary>${navigationSummary(st.engine)}</div>
    </nav>
    <main class="wb-main">
      <header class="wb-head" data-tauri-drag-region><span class="wb-head-title">${title}</span><div class="wb-status" data-wb-status>${workbenchStatus(st.engine)}</div><div class="wb-head-actions"><button type="button" class="mini-btn" data-wb-quick aria-keyshortcuts="Meta+K Control+K">前往 <kbd>⌘K</kbd></button><button type="button" class="mini-btn" data-wb-hide>收起窗口</button></div></header>
      <div tabindex="-1" aria-label="工作台内容" class="wb-content${selected === 'overview' ? ' wb-overview' : ''}">${content}</div>
    </main>
  </div>`;
  retainWorkbenchControls(workbenchPagesCache.retainedControls);
  revealNavigationItem(root.querySelector('.wb-nav-links'));
  pageMotion(root, selected, motionSurface);
  if (!restored || restored.querySelector('[data-provider-root], [data-todo-root], [data-remote-root], [data-tasks-root], [data-sessions-root]')?.dataset.pageReady !== 'true') {
  if (selected === 'tokenAnalytics') { hydrateReport(); bindToolBudgets(root); }
  if (selected === 'provider') hydrateProvider();
  if (selected === 'sessions') hydrateSessions(id => { selectTask(id); st.workbenchPage='tasks'; st.route='list'; renderWorkbench(); }, () => getState().windowVisible !== false);
  if (selected === 'tasks') hydrateTasks(() => getState().windowVisible !== false);
  if (selected === 'windows') hydrateWindowLayout();
  if (selected === 'workspaces') hydrateWorkspaces(openWorkspaceStep);
  if (selected === 'overview' || selected === 'todo') hydrateTodo();
  if (selected === 'remote') hydrateRemote();
  if (!restored && selected === 'settings') bindSettings();
  if (!restored && selected === 'agents') bindAgents();
  }
  bindWorkbench();
  const quick=bindQuickNavigation({items:workbenchPages.map(([key,label])=>({key,label,aliases:({provider:'模型 接口 档位 MCP Skills 提示词',windows:'布局 排列',tokenAnalytics:'token tokens 费用 趋势',todo:'todos 待办',sessions:'会话 来源 历史',tasks:'任务 看板 运行 结果 问题',workspaces:'项目 组合',settings:'偏好 外观 通知'}[key]??'')})).concat([{key:'report',label:'用量报告',aliases:'导出 export Markdown CSV'}]),current:()=>getState().workbenchPage,navigate:key=>{if(key==='report'){getState().workbenchPage='report';renderWorkbench();}else if(key==='todo'){getState().workbenchPage='todo';getState().route='list';renderWorkbench();root.querySelector('[data-wb-task-view][data-wb-nav="todo"]')?.focus({preventScroll:true});}else root.querySelector(`.wb-nav-item[data-wb-nav="${key}"]`)?.click();}});
  const quickButton=root.querySelector('[data-wb-quick]');if(quickButton)quickButton.onclick=quick.open;
  renderTaskAttentionOnly();
  if (restored && selected === 'tasks') refreshTasks();
  if (restored && selected === 'sessions') refreshSessions();
  if (restored && selected === 'overview') renderWorkbenchMonitorOnly();
  workspaceFlow.route(selected);
  renderWorkspaceBanner();
  root.querySelector('[data-workspaces-root]')?.renderWorkspaceProgress?.();
  focusWorkspaceTarget();
  if(showReport)openUsageReport(root);
  root.querySelectorAll('[data-wb-nav]').forEach((button) => {
    button.onclick = () => {
      if (st.workbenchPage === button.dataset.wbNav) return;
      st.workbenchPage = button.dataset.wbNav;
      st.route = st.workbenchPage === 'tokenAnalytics' ? 'tokenAnalytics' : 'list';
      renderWorkbench();
      if(button.dataset.wbNav!=='report') {
        const selector=button.hasAttribute('data-wb-task-view') ? `[data-wb-task-view][data-wb-nav="${st.workbenchPage}"]` : `.wb-nav-item[data-wb-nav="${st.workbenchPage==='todo'?'tasks':st.workbenchPage}"]`;
        root.querySelector(selector)?.focus({preventScroll:true});
      }
    };
  });
}

let workspaceTarget=null;
function openWorkspaceStep(step,recovery=null){
  if(!step.available)return;
  const page={project:'tasks',profile:'provider',layout:'windows',tool:'agents'}[step.kind];if(!page)return;
  workspaceTarget={...step,recovery};
  if(step.kind==='project')selectTaskProject(step.target_id);
  if(step.kind==='layout'){if(recovery)selectLayoutRecovery(recovery.recovery_id);else selectLayoutRule(step.target_id);}
  getState().workbenchPage=page;getState().route='list';renderWorkbench();
}
function focusWorkspaceTarget(){
  if(!workspaceTarget)return;
  const step=workspaceTarget;
  if(getState().workbenchPage!==({project:'tasks',profile:'provider',layout:'windows',tool:'agents'}[step.kind])){workspaceTarget=null;return;}
  if(step.kind==='project'||step.kind==='layout'){
    if(step.kind==='layout'){const root=document.querySelector('[data-layout-root]');if(step.recovery)root?.focusWorkspaceRecovery?.();else root?.selectWorkspaceRule?.();}
    workspaceTarget=null;return;
  }
  const selector=step.recovery?'[data-restore]':step.kind==='profile'?'[data-profile-row]':'[data-agent-toggle]';
  const rows=[...document.querySelectorAll(selector)];
  const target=rows.find(e=>(step.recovery?e.dataset.restore:step.kind==='profile'?e.dataset.profileRow:e.dataset.agentToggle)===(step.recovery?step.recovery.recovery_id:step.target_id));
  if(!target){
    const box=document.querySelector(step.kind==='profile'?'[data-provider-root]':'[data-agents-root]');
    if(box&&(step.kind==='tool'||box.dataset.pageReady==='true')){
      workspaceTarget=null;let note=box.querySelector('[data-workspace-target-status]');
      if(!note){note=document.createElement('p');note.dataset.workspaceTargetStatus='';note.className='sb-note';note.setAttribute('role','status');box.prepend(note);}
      note.textContent=step.recovery?'恢复备份未在当前列表中，请刷新核对。':step.kind==='profile'?'所选档位未在当前列表中，请刷新核对。':'所选工具未在管理列表中，请核对安装与监控设置。';
    }
    return;
  }
  workspaceTarget=null;
  for(let parent=target.parentElement;parent;parent=parent.parentElement)if(parent.tagName==='DETAILS')parent.open=true;
  const focus=step.recovery?target:step.kind==='profile'?target.querySelector('[data-switch]'):target;
  if(focus&&!focus.disabled)focus.focus({preventScroll:true});else{target.tabIndex=-1;target.focus({preventScroll:true});}
  target.scrollIntoView({block:'nearest',behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'auto':'smooth'});
}

function renderWorkspaceBanner(){
 const main=document.querySelector('.wb-main');if(!main)return;
 let banner=main.querySelector('[data-workspace-flow]');
 if(!banner){banner=document.createElement('div');banner.className='workspace-flow';banner.dataset.workspaceFlow='';main.querySelector('[data-wb-status]').before(banner);}
 const session=workspaceFlow.session,intent=workspaceFlow.intent;
 banner.hidden=!session||!intent||workspaceRoute(intent.kind)!==getState().workbenchPage;
 main.querySelector('.wb-head-title').hidden=!banner.hidden;
 if(banner.hidden)return;
 const receipt=workspaceFlow.receipt(intent.kind);
 banner.innerHTML=`<span><strong>${esc(session.name)}</strong><small>${workspaceFlow.pending?'正在处理分项':!workspaceWritable(intent.kind)?'查看关联内容':intent.recovery?'核对后恢复':receipt?esc(workspaceReceiptLabel(receipt.state)):'预览后应用'}</small></span><button type="button" class="mini-btn" data-workspace-return>返回组合</button>`;
 banner.querySelector('button').onclick=()=>{getState().workbenchPage='workspaces';getState().route='list';renderWorkbench();document.querySelector('[data-workspace-close]')?.focus({preventScroll:true});};
}
workspaceFlow.onChange=()=>{renderWorkspaceBanner();document.querySelector('[data-workspaces-root]')?.renderWorkspaceProgress?.();};
const verifyWorkspace=args=>invoke('workspace_preview',args);

function workbenchSummary(eng) {
  if (!eng) return '<div><strong>—</strong><span>在线智能体</span></div><div><strong>—</strong><span>24h tokens</span></div><div><strong>—</strong><span>待确认</span></div>';
  const running = eng.snapshots.filter(isVisible);
  const count = running.length;
  const attention = running.filter(s => s.level === 'attention').length;
  return `<div><strong>${count}</strong><span>在线智能体</span></div><div><strong>${compact(eng.grand_total.tokens24h)}</strong><span>24h tokens</span></div><div><strong>${attention}</strong><span>待确认</span></div>`;
}

/** 报告面板的交互：生成（两种格式）与复制。 */
function bindReportPanel() {
  document.querySelectorAll('[data-report-panel]').forEach(panel=>{if(!panel.closest('[data-page-outgoing]'))bindReport(panel);});
}

/**
 * 只重画监控那一块。
 *
 * **整页重画会毁掉别的东西**：Provider 面板里正在填的表单、待办的输入框光标，
 * 每 2 秒被冲一次就没法用了。所以推送只动监控列表的 innerHTML。
 * 同一个坑在侧边栏上踩过一次（那里是「只有列表页随推送重画」）。
 */
export function renderWorkbenchMonitorOnly() {
  renderNavSummaryOnly();
  renderTaskAttentionOnly();
  const box = document.querySelector('[data-wb-monitor]');
  const eng = getState().engine;
  const summary = document.querySelector('[data-wb-summary]');
  if (summary) summary.innerHTML = workbenchSummary(eng);
  const statusEl = document.querySelector('[data-wb-status]');
  if (statusEl) statusEl.textContent = workbenchStatus(eng);
  if (!box) return;
  // 详情中的报告是异步注水的，周期采样不能把它重新打回空壳。
  if (getState().route.startsWith('agentDetail:') && box.dataset.detailRoute === getState().route && box.querySelector('[data-report-root]')) return;
  box.innerHTML = workbenchMonitor(eng);
  box.dataset.detailRoute = getState().route;
  bindWorkbenchAgentClicks();
}

/** 点 Agent 进详情。监控列表在整页与局部重画两条路上都要绑，只写一处。 */
function bindWorkbenchAgentClicks() {
  const root = document.getElementById('root');
  root.querySelectorAll('[data-open-agent]').forEach(button => {
    button.onclick = () => openAgentSession(button, button.dataset.openAgent, null, button.dataset.expectedUrl || null);
  });
  root.querySelectorAll('[data-agent]').forEach((el) => {
    el.onclick = () => {
      getState().route = `agentDetail:${el.dataset.agent}`;
      renderWorkbenchMonitorOnly();
      hydrateReport();
    };
  });
  root.querySelectorAll('[data-back]').forEach((el) => {
    el.onclick = () => { getState().route = 'list'; renderWorkbenchMonitorOnly(); };
  });
}

/** 工作台自己的交互：关掉自己、报告面板、点 Agent 进详情。 */
function bindWorkbench() {
  const root = document.getElementById('root');
  const hide = root.querySelector('[data-wb-hide]');
  if (hide) hide.onclick = () => invoke('hide_workbench').catch(() => {});
  bindReportPanel();
  bindWorkbenchAgentClicks();
}

export function renderSidebar() {
  const focusKey = document.activeElement?.dataset?.nav;
  const scrollTop = document.querySelector('.sb-body')?.scrollTop ?? 0;
  const st = getState();
  const eng = st.engine ?? {
    snapshots: [], grand_total: {},
    latest_event: null, any_working: false, has_attention: false,
  };
  const running = eng.snapshots.filter(isVisible);
  const attention = eng.snapshots.filter((snap) => snap.level === 'attention').length;

  const nav = [
    { key: 'list', label: '监控', count: running.length },
    { key: 'tokenAnalytics', label: 'Token 用量', count: 0 },
    { key: 'provider', label: 'Codex 档位', count: 0 },
    // 角标是**未完成**数（不是总条数）：勾掉最后一条之后角标就该消失
    { key: 'todo', label: '待办', count: st.todosPending ?? 0, badge: 'todo' },
    // 03-approach §3：首层极简，重页面从这里进
    { key: 'settings', label: '设置', count: 0 },
    { key: 'remote', label: '远程通知', count: 0 },
    { key: 'agents', label: '监控管理', count: 0 },
  ];
  const route = ['list', 'tokenAnalytics', 'provider', 'todo', 'settings', 'remote', 'agents']
    .includes(st.route) ? st.route : 'list';

  let body;
  if (route === 'settings') {
    body = pageSettings();
  } else if (route === 'remote') {
    body = pageRemote();
  } else if (route === 'agents') {
    body = pageAgents();
  } else if (route === 'todo') {
    body = pageTodo();
  } else if (route === 'provider') {
    body = pageProvider();
  } else if (route === 'tokenAnalytics') {
    body = pageAnalytics(eng);
  } else if (!st.engine) {
    body = '<div class="sb-empty" role="status">等待采样</div>';
  } else if (running.length === 0) {
    body = '<div class="sb-empty">暂无在线智能体</div>';
  } else {
    body = running
      // 参数命名成 `snap`（而不是 `s`）是**有意的**：`models.rs` 有一条跨文件哨兵，
      // 它扫 views.js 里所有的 `snap.<字段>` 并断言快照 JSON 里确实有那个字段。
      // 用别的名字就绕过了那条哨兵，字段改名时会静默失效。
      .map((snap) => {
        // 与灵动岛**同一份**显示模型：状态怎么说、用量怎么缩写、没取到写什么，
        // 两处不会再各说各话（排版仍各自不同）
        const model = agentRowModel(snap);
        const tokens = model.tokensText === '—' ? '—' : `${model.tokensText} tokens`;
        const detail = model.actionText || model.activityText;
        return `<button type="button" class="sb-agent" data-level="${esc(model.level)}" data-agent="${esc(model.id)}">
          ${agentIcon(snap)}
          <span class="name">${escapeHtml(model.name)}</span>
          <span class="tokens" style="color:${model.statusColor}">${tokens}</span>
          <span class="meta"><span class="sb-agent-status" style="color:${model.statusColor}">${escapeHtml(model.statusText)}</span>${detail && detail !== model.statusText ? `<span class="sb-agent-action">${escapeHtml(detail)}</span>` : ''}</span>
        </button>`;
      })
      .join('');
  }

  const header = route === 'settings'
    ? { t: '设置', s: '修改自动保存' }
    : route === 'remote'
    ? { t: '远程通知', s: '密钥只进系统钥匙串' }
    : route === 'agents'
    ? { t: '监控管理', s: '仅控制监控，应用继续运行' }
    : route === 'tokenAnalytics'
    ? { t: 'Token 用量', s: '净消耗 · 不含缓存读取' }
    : route === 'provider'
      ? { t: 'Codex 档位', s: '管理模型与接口' }
      : route === 'todo'
        // 这条分支是**看着截图补的**：待办页原先落到下面的 else，表头写着「智能体 / 全部正常」。
        // 静态检查与冒烟都发现不了——它们只看「有没有报错」。
        ? { t: '待办', s: `未完成 ${st.todosPending ?? 0} 条` }
        : { t: '智能体', s: !st.engine ? '等待采样' : attention > 0 ? `${attention} 个等待确认` : '全部正常' };

  const root = document.getElementById('root');
  rememberPage(root, route, '.sb-body');
  root.innerHTML = `
    <div class="sb">
      <nav class="sb-nav" aria-label="主导航">
        <div class="sb-brand">AgentIsland</div>
        ${nav
          .map(
            (item) => `<button type="button" class="sb-item${item.key === route ? ' is-active' : ''}${item.disabled ? ' is-disabled' : ''}"
              ${item.disabled ? 'disabled' : `data-nav="${item.key}"`} aria-current="${item.key === route ? 'page' : 'false'}">
              ${navigationIcon(({ list: 'terminal', tokenAnalytics: 'chart', provider: 'sliders', todo: 'check', settings: 'gear', remote: 'bell', agents: 'terminal' })[item.key])}<span>${escapeHtml(item.label)}</span>${item.text
                ? `<span class="sb-count" data-nav-text="${escapeHtml(item.badge ?? '')}">${escapeHtml(item.text)}</span>`
                : item.count ? `<span class="sb-count" ${item.badge ? `data-nav-count="${item.badge}"` : ''}>${item.count}</span>` : ''}
            </button>`,
          )
          .join('')}
        <div class="sb-nav-footer" data-nav-summary>${navigationSummary(st.engine)}</div>
      </nav>
      <main class="sb-main">
        <div class="sb-head"><div class="t">${header.t}</div><div class="s">${header.s}</div></div>
        <div class="sb-body">${route === 'list' ? `<div data-task-attention-slot>${taskAttentionHtml(getTaskAttention(), eng)}</div>` : ''}${body}</div>
      </main>
    </div>`;


  bindTaskAttention(root);
  pageMotion(root, route, '.sb-body');
  if (focusKey) root.querySelector(`[data-nav="${focusKey}"]`)?.focus({ preventScroll: true });
  root.querySelector('.sb-body').scrollTop = scrollTop;
  if (route === 'settings') bindSettings();
  if (route === 'agents') bindAgents();
  if (route === 'remote') hydrateRemote();

  // 量一次布局（在内容就位之后：把度量放在注水之前只会量到「加载中…」的骨架，
  // 我第一版就是这么量的，于是每个路由的数字都一模一样、看起来「都没问题」）。
  scheduleLayoutLog();

  root.querySelectorAll('[data-nav]').forEach((el) => {
    el.onclick = async () => {
      st.route = el.dataset.nav;
      renderSidebar();
      root.querySelector('.sb-body').scrollTop = 0;
      if (st.route === 'tokenAnalytics') await hydrateReport();
      if (st.route === 'provider') await hydrateProvider();
      if (st.route === 'todo') await hydrateTodo();
      if (st.route === 'settings') bindSettings();
      if (st.route === 'agents') bindAgents();
      if (st.route === 'remote') await hydrateRemote();
    };
  });
  root.querySelectorAll('[data-agent]').forEach((el) => {
    el.onclick = () => {
      st.route = `agentDetail:${el.dataset.agent}`;
      // **不借 `renderCard()`**：那个函数会把内容包进灵动岛的 `.card dock-*` 外壳里，
      // 在侧边栏里就是一个尺寸与形状都不属于这里的盒子。页面函数本身是共用的
      // （`pageAgentDetail` + `hydrateReport`），只有外壳不同。
      renderSidebarDetail();
      hydrateReport();
    };
  });
}

/// 侧边栏里的 Agent 详情：同一份页面函数，铺进内容区
export function renderSidebarDetail() {
  const st = getState();
  const eng = st.engine ?? { snapshots: [] };
  const agentId = st.route.startsWith('agentDetail:') ? st.route.split(':')[1] : '';
  const body = document.querySelector('.sb-body');
  if (!body) return;
  rememberPage(document.getElementById('root'), st.route, '.sb-body');
  body.innerHTML = `<div class="sb-page">${pageAgentDetail(eng, agentId)}</div>`;
  pageMotion(document.getElementById('root'), st.route, '.sb-body');
  body.querySelector('[data-back]')?.addEventListener('click', () => { st.route = 'list'; renderSidebar(); });
}

function escapeHtml(text) {
  return String(text ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}

// MARK: - Codex 档位页（Phase 2）

/// 档位页骨架。数据要读 `config.toml` 与档位库（异步），所以先出骨架、
/// 再由 `hydrateProvider()` 填内容——与报表页同一套路。


// MARK: Agent 启停列表

/// 逐项启停。**语义与 macOS 相反，要在这里说清**：
/// macOS 存的是「启用集合」（空集 = 全部关掉），Rust 存的是「禁用集合」（空集 = 全部开着）。
/// 两种都能表示同一个用户意图，但**空集的意思正好相反**——所以界面上
/// 显式写出这一句，而不是让用户对着两个空列表猜。
export function pageAgents() {
  const st = getState();
  const eng = st.engine ?? { snapshots: [] };
  const disabled = new Set(st.settings?.disabled_agents ?? []);
  const seen = new Set(eng.snapshots.map((s) => s.id));
  const rows = eng.snapshots
    .map((snap) => {
      const off = disabled.has(snap.id);
      return `<label class="sb-agent-toggle">
        <input type="checkbox" data-agent-toggle="${escapeHtml(snap.id)}"${off ? '' : ' checked'}>
        ${agentIcon(snap)}
        <span class="name">${escapeHtml(snap.name)}</span>
        <span class="meta">${off ? '已关' : '开着'}</span>
      </label>`;
    })
    .join('');
  // 引擎这一拍没出现、但被关掉的档案也要列出来，否则「关掉了就再也找不回来」
  const orphans = (st.settings?.disabled_agents ?? [])
    .filter((id) => !seen.has(id))
    .map((id) => `<label class="sb-agent-toggle">
        <input type="checkbox" data-agent-toggle="${escapeHtml(id)}">
        ${agentIcon({ id })}
        <span class="name">${escapeHtml(id)}</span>
        <span class="meta">未监控 · 未检测到</span>
      </label>`)
    .join('');

  return `<div class="sb-page" data-agents-root>
    <div class="sb-note">选择要监控的智能体。关闭监控后，应用继续运行。</div>
    <div class="sb-hint">随时可重新开启监控。</div>
    ${rows || '<div class="sb-empty">暂无可监控的智能体</div>'}
    ${orphans}
  </div>`;
}

export function bindAgents() {
  const st = getState();
  st.settings = st.settings ?? {};
  st.settings.disabled_agents = st.settings.disabled_agents ?? [];
  document.querySelectorAll('[data-agent-toggle]').forEach((el) => {
    el.addEventListener('change', async () => {
      const id = el.dataset.agentToggle;
      const previousDisabled = [...st.settings.disabled_agents];
      const set = new Set(st.settings.disabled_agents);
      if (el.checked) set.delete(id); else set.add(id);
      st.settings.disabled_agents = [...set];
      try { await saveSettings({ disabled_agents: st.settings.disabled_agents }); }
      catch (error) {
        st.settings.disabled_agents = previousDisabled;
        el.checked = !previousDisabled.includes(id);
        el.closest('.sb-agent-toggle')?.appendChild(notice(`保存失败：${error}`));
        return;
      }
      // 引擎下一拍就会按新集合过滤（状态是 `engine://tick` 推来的）。
      // 这里立刻重画是为了不让人对着一个已经改了、看起来却没动的界面发愣。
      if (isWorkbench()) renderWorkbench(); else renderSidebar();
    });
  });
}

// MARK: 远程通知（三通道 + 钥匙串密钥 + 发送预览）

/// 三个通道的**字段表**：显示名、键、控件类型只有这一份。
/// 通道枚举的取值（`ntfy` / `customHTTP` / `smtpEmail`）与 Rust `remote::Channel::as_str`
/// 同值——写错一个，界面就写进了一个 Rust 认不出的通道，而 `normalized()` 会把它
/// 悄悄回落成 ntfy，用户看到的是「我选了却没生效」而没有任何报错。
const REMOTE_CHANNELS = [
  { kind: 'feishuBot', label: '飞书群机器人', secretLabel: 'Webhook 地址', help: '飞书群设置 → 群机器人 → 添加自定义机器人，复制 Webhook 即可。若使用关键词校验，请允许 AgentIsland；签名校验在高级设置中填写。', fields: [] },
  { kind: 'wechatPushPlus', label: '微信 · PushPlus', secretLabel: 'PushPlus Token', help: '关注 PushPlus 公众号，在 PushPlus 官网获取 Token。通知通过公众号发到微信，受平台额度与订阅限制。', fields: [] },
  { kind: 'qqPushPlus', label: 'QQ · PushPlus 机器人', secretLabel: 'PushPlus Token', help: '先在 PushPlus 个人中心 → 渠道配置 → QQ 机器人完成绑定，然后粘贴 Token。默认发给自己，群推送可在高级设置填写群配置编码。', fields: [] },
  { kind: 'qqOneBot', label: 'QQ 群 · OneBot 11（已有机器人）', secretLabel: '访问令牌（可选）', help: '需要已登录并运行的 OneBot 11 群机器人，填写它的 HTTP 服务地址与群号。AgentIsland 不负责登录 QQ。', fields: [
    { key: 'url_template', label: '机器人地址', ph: 'http://127.0.0.1:3000' },
    { key: 'topicOrURL', label: 'QQ 群号', ph: '接收通知的群号' } ] },
  { kind: 'ntfy', label: 'ntfy 推送', help: '填写主题名，并在手机 ntfy 应用订阅同一主题。', fields: [
    { key: 'topicOrURL', label: '主题名', ph: 'my-agentisland-topic' } ] },
  { kind: 'smtpEmail', label: '邮箱 · SMTP', secretLabel: '密码 / 授权码', help: '填写邮箱的 SMTP 服务器、发信账号与收件人；使用 465 加密端口。部分邮箱需要单独生成授权码。', fields: [
    { key: 'smtp_host', label: 'SMTP 服务器', ph: 'smtp.qq.com' },
    { key: 'smtp_user', label: '发信邮箱' },
    { key: 'smtp_to', label: '收件邮箱' } ] },
  { kind: 'customHTTP', label: '自定义 HTTP（高级）', secretLabel: '密钥', help: '适用于其他服务。地址与正文模板在高级设置中配置。', fields: [
    { key: 'url_template', label: '地址', ph: 'https://example.com/send?key={key}' },
    { key: 'body_template', label: '正文模板', ph: '{title}\n{body}' },
    { key: 'useJSONBody', label: '正文用 JSON', type: 'bool' } ] },
];

/// 远程通知页外壳。**能力边界逐字用后端给的那段**（与档位页同一条纪律：
/// 界面不自己编一句「我们支持什么」——那份文案是约束，编错了就是骗）。
export function pageRemote() {
  return `<div class="sb-page" data-remote-root>
    <div class="sb-empty">加载中…</div>
  </div>`;
}

export async function hydrateRemote() {
  const root = document.querySelector('[data-remote-root]');
  if (!root) return;
  const currentRequest = pageRequest(root);
  const remote = await invoke('remote_status').catch(() => null);
  if (!currentRequest()) return;
  if (!remote) {
    root.innerHTML = '<div class="sb-empty">读不到远程通知状态（命令没接上？）</div>';
    return;
  }
  const st = getState();
  st.remoteStatus = remote;
  const channel = REMOTE_CHANNELS.find((c) => c.kind === remote.kind) ?? REMOTE_CHANNELS[0];
  const cfg = (st.settings?.remote_channels ?? {})[channel.kind] ?? {};
  const policy = st.settings?.remote_policy ?? remote.policy ?? {};

  // ③ 未配齐 / 不安全端点 / 静默中 / 在场判定——每条都是**独立**的一行，
  // 不合成一句「有问题」：用户要能分辨「没配」与「配了但不安全」
  const notices = [];
  if (remote.readiness) notices.push(`未配齐：${remote.readiness}`);
  if (remote.plaintextSecret) notices.push(`配置警告：${remote.plaintextSecret}`);
  if (remote.insecureEndpoint) notices.push(`端点不安全：${remote.insecureEndpoint}`);
  if (remote.quietNow) notices.push('此刻落在静默时段内');
  if (remote.awayNow) notices.push(`在场判定：${remote.awayReason}`);
  if (remote.unrecognizedKind) notices.push(`设置里的通道「${remote.unrecognizedKind}」认不出，已回落到 ${remote.label}`);

  const policyToggle = (key, label, fallback = false) => `<label class="sb-set"><span class="sb-set-label">${label}</span><span class="sb-set-ctl"><input type="checkbox" data-remote-policy="${key}"${(policy[key] ?? fallback) ? ' checked' : ''}></span></label>`;
  const secretControl = channel.secretLabel ? `<label class="sb-set"><span class="sb-set-label">${channel.secretLabel}</span><span class="sb-set-ctl"><input type="password" data-remote-secret placeholder="留空保留已保存的连接" autocomplete="off" spellcheck="false"></span></label><div class="sb-hint">${remote.credentialStored ? '连接凭据已保存；留空保留。' : '填好后点击保存连接。'} 凭据只存系统钥匙串。</div>` : '';
  root.innerHTML = `
    <div class="sb-group remote-intro"><div class="sb-group-title">把重要进展送到你身边</div><div class="sb-hint">选一个渠道，填好连接信息。默认只发送 Agent 名称与状态。</div></div>
    <div class="sb-group">
      <div class="sb-group-title">接收方式</div>
      <label class="sb-set"><span class="sb-set-label">通知渠道</span><span class="sb-set-ctl"><select data-remote-kind>${REMOTE_CHANNELS.map(c => `<option value="${c.kind}"${c.kind === remote.kind ? ' selected' : ''}>${escapeHtml(c.label)}</option>`).join('')}</select></span></label>
      <div class="sb-hint remote-channel-help">${escapeHtml(channel.help)}</div>
      ${channel.kind === 'customHTTP' ? '' : channel.fields.map(f => remoteField(channel.kind, f, cfg)).join('')}
      ${secretControl}
      ${notices.map(n => `<div class="sb-hint sb-warn">${escapeHtml(n)}</div>`).join('')}
    </div>
    <div class="sb-group">
      <div class="sb-group-title">通知哪些进展</div>
      ${policyToggle('master_enabled', '开启远程通知')}
      ${policyToggle('send_completed', '任务完成', true)}
      ${policyToggle('send_attention', '需要我确认', true)}
      ${policyToggle('send_cost_spike', '资源 / 消耗告警', true)}
      <div class="sb-foot">
        ${channel.secretLabel ? '<button type="button" class="mini-btn" data-remote-save-secret>保存连接</button>' : ''}
        <button type="button" class="mini-btn" data-remote-test>发送测试</button>
      </div>
      <div class="sb-hint">开启远程通知后可测试；测试会跳过静默、离开与节流策略。平台受理不代表手机已收到或消息已读。</div>
      <div data-remote-out class="sb-note" role="status" aria-live="polite"></div>
    </div>
    <details class="sb-group remote-advanced" data-remote-advanced>
      <summary class="sb-group-title">高级设置 <span>模板、签名与发送策略</span></summary>
      ${channel.kind === 'customHTTP' ? channel.fields.map(f => remoteField(channel.kind, f, cfg)).join('') : ''}
      ${channel.kind === 'qqPushPlus' ? remoteField(channel.kind, { key: 'topicOrURL', label: '群配置编码（可选）', ph: '留空发给自己' }, cfg) : ''}
      ${channel.kind === 'feishuBot' ? '<label class="sb-set"><span class="sb-set-label">签名密钥（可选）</span><span class="sb-set-ctl"><input type="password" data-remote-signing autocomplete="off" placeholder="启用签名校验时填写"></span></label><div class="sb-hint">修改签名时需同时填写 Webhook，再保存连接。两项一起存入钥匙串。</div>' : ''}
      <label class="sb-set"><span class="sb-set-label">附带最后一条动作</span><span class="sb-set-ctl"><input type="checkbox" data-remote-cfg="include_action_detail" data-kind="${channel.kind}"${cfg.include_action_detail ? ' checked' : ''}></span></label>
      <div class="sb-hint">开启后会把命令内容与文件路径一起送出。</div>
      <label class="sb-set"><span class="sb-set-label">同事件节流</span>
        <span class="sb-set-ctl"><input type="number" data-remote-policy="throttle_seconds" min="15" max="3600"
          value="${escapeHtml(String(policy.throttle_seconds ?? 90))}"><span class="sb-unit">秒</span></span></label>
      <label class="sb-set"><span class="sb-set-label">静默时段起</span>
        <span class="sb-set-ctl"><input type="text" data-remote-policy="quiet_start" placeholder="22:00" value="${escapeHtml(policy.quiet_start ?? '')}"></span></label>
      <label class="sb-set"><span class="sb-set-label">静默时段止</span>
        <span class="sb-set-ctl"><input type="text" data-remote-policy="quiet_end" placeholder="08:00" value="${escapeHtml(policy.quiet_end ?? '')}"></span></label>
      <label class="sb-set"><span class="sb-set-label">只在人不在时发</span>
        <span class="sb-set-ctl"><input type="checkbox" data-remote-policy="only_when_away"${policy.only_when_away ? ' checked' : ''}></span></label>
      <label class="sb-set"><span class="sb-set-label">无输入判定</span>
        <span class="sb-set-ctl"><input type="number" data-remote-policy="away_idle_seconds" min="30" max="3600"
          value="${escapeHtml(String(policy.away_idle_seconds ?? 120))}"><span class="sb-unit">秒</span></span></label>
      <div class="sb-hint">macOS 根据显示器睡眠和无输入时长判断离开；锁屏信号暂未接入。信号不可用时按已离开放行。</div>

      <div class="sb-foot"><button type="button" class="mini-btn" data-remote-preview>查看发送内容</button>${channel.secretLabel ? '<button type="button" class="mini-btn" data-remote-del-secret>删除已保存的凭据</button>' : ''}</div>
      <div class="sb-hint">钥匙串条目 <code>${escapeHtml(remote.secretName)}</code></div>
      <div class="sb-note">${escapeHtml(remote.limitations ?? '')}</div>
      <div data-remote-history class="sb-note"></div><button type="button" class="mini-btn" data-remote-refresh>刷新发送记录</button>
    </details>`;
  root.dataset.pageReady = 'true';
  bindRemote();
  hydrateRemoteHistory();
  scheduleLayoutLog();
}

function remoteField(kind, field, cfg) {
  const id = `${kind}.${field.key}`;
  if (field.type === 'bool') {
    return `<label class="sb-set"><span class="sb-set-label">${escapeHtml(field.label)}</span>
      <span class="sb-set-ctl"><input type="checkbox" data-remote-cfg="${escapeHtml(field.key)}" data-kind="${kind}"${cfg[field.key] ? ' checked' : ''}></span></label>`;
  }
  const attrs = field.type === 'number' ? ` type="number" min="${field.min}" max="${field.max}"` : ' type="text"';
  return `<label class="sb-set"><span class="sb-set-label">${escapeHtml(field.label)}</span>
    <span class="sb-set-ctl"><input${attrs} data-remote-cfg="${escapeHtml(field.key)}" data-kind="${kind}"
      placeholder="${escapeHtml(field.ph ?? '')}" value="${escapeHtml(String(cfg[field.key] ?? ''))}"></span></label>`;
}

export async function hydrateRemoteHistory() {
  const box = document.querySelector('[data-remote-history]');
  if (!box) return;
  try {
    const recent = await invoke('remote_recent');
    if (!box.isConnected) return;
    box.innerHTML = recent.length ? recent.map(entry => `<div class="sb-hint">${escapeHtml(entry.title)} · ${escapeHtml(entry.text)}</div>`).join('') : '<div class="sb-hint">尚无发送记录</div>';
  } catch (error) { if (box.isConnected) box.textContent = `读取记录失败：${error}`; }
}

function bindRemote() {
  const st = getState();
  st.settings = st.settings ?? {};
  st.settings.remote_channels = st.settings.remote_channels ?? {};
  st.settings.remote_policy = st.settings.remote_policy ?? {};

  let pendingSave = Promise.resolve(true);
  const persist = (out) => {
    const patch = structuredClone({ remote_kind: st.settings.remote_kind, remote_channels: st.settings.remote_channels, remote_policy: st.settings.remote_policy });
    pendingSave = pendingSave.then(async () => {
    try {
      await saveSettings(patch);
      out.innerHTML = '<div class="sb-hint">已保存</div>';
      return true;
    } catch (error) {
      out.innerHTML = `<div class="sb-hint sb-warn">保存失败：${escapeHtml(String(error))}</div>`;
      return false;
    }
    });
    return pendingSave;
  };
  const out = document.querySelector('[data-remote-out]') ?? document.createElement('div');

  document.querySelector('[data-remote-refresh]')?.addEventListener('click', hydrateRemoteHistory);
  document.querySelector('[data-remote-test]')?.addEventListener('click', async (event) => {
    const button = event.currentTarget;
    button.disabled = true;
    try {
      if (!await pendingSave) return;
      if (!st.settings.remote_policy.master_enabled) { out.textContent = '请先开启远程通知，再发送测试。'; return; }
      if (document.querySelector('[data-remote-secret]')?.value.trim() || document.querySelector('[data-remote-signing]')?.value.trim()) { out.textContent = '请先保存连接，再发送测试。'; return; }
      out.textContent = '正在测试发送…';
      out.textContent = await invoke('remote_send_test');
    }
    catch (error) { out.textContent = `发送失败：${error}`; }
    finally { button.disabled = false; await hydrateRemoteHistory(); }
  });

  document.querySelector('[data-remote-kind]')?.addEventListener('change', async (e) => {
    const previous = st.settings.remote_kind;
    st.settings.remote_kind = e.target.value;
    if (!await persist(out)) { st.settings.remote_kind = previous; e.target.value = previous; return; }
    await hydrateRemote(); // 换通道要重画字段：三个通道的键不一样
  });

  document.querySelectorAll('[data-remote-cfg]').forEach((el) => {
    el.addEventListener('change', async () => {
      const kind = el.dataset.kind;
      const key = el.dataset.remoteCfg;
      st.settings.remote_channels[kind] = st.settings.remote_channels[kind] ?? {};
      st.settings.remote_channels[kind][key] =
        el.type === 'checkbox' ? el.checked : (el.type === 'number' ? Number(el.value) : el.value);
      await persist(out);
    });
  });

  document.querySelectorAll('[data-remote-policy]').forEach((el) => {
    el.addEventListener('change', async () => {
      const key = el.dataset.remotePolicy;
      st.settings.remote_policy[key] =
        el.type === 'checkbox' ? el.checked : (el.type === 'number' ? Number(el.value) : el.value);
      await persist(out);
    });
  });

  // 密钥：只往钥匙串写，**永不读回**（后端刻意没有读回命令）
  document.querySelector('[data-remote-save-secret]')?.addEventListener('click', async () => {
    const input = document.querySelector('[data-remote-secret]');
    if (!await pendingSave) return;
    const raw = input?.value.trim() ?? '';
    if (!raw) { out.textContent = '留空会保留现有连接。需要更换时填写凭据，需要删除时使用高级设置。'; return; }
    const value = st.settings.remote_kind === 'feishuBot' ? JSON.stringify({ webhook: raw, signingSecret: document.querySelector('[data-remote-signing]')?.value.trim() ?? '' }) : raw;
    const result = await invoke('remote_secret_set', { value })
      .catch((e) => ({ kind: 'Failed', reason: String(e) }));
    if (result.kind === 'ok') {
      input.value = '';
      const signing = document.querySelector('[data-remote-signing]');
      if (signing) signing.value = '';
      out.innerHTML = '<div class="sb-hint">密钥已写入钥匙串</div>';
      await hydrateRemote();
    } else {
      // 写入被系统拒绝时**把原因显示出来**——本应用是 ad-hoc 签名，
      // 弹窗点「始终允许」这一步用户必须自己做得到
      out.innerHTML = `<div class="sb-hint sb-warn">写入被拒：${escapeHtml(result.reason ?? '（无原因）')}</div>`;
    }
  });
  document.querySelector('[data-remote-del-secret]')?.addEventListener('click', async () => {
    if (!await pendingSave) return;
    const deleted = await invoke('remote_secret_delete').catch(() => false);
    if (!deleted) { out.textContent = '删除未完成：凭据不存在或系统拒绝访问。'; return; }
    await hydrateRemote();
  });
  document.querySelector('[data-remote-preview]')?.addEventListener('click', async () => {
    const preview = await invoke('remote_preview', { args: { kind: 'attention', agentName: 'AgentIsland', seconds: 0 } })
      .catch(() => null);
    if (!preview) { out.innerHTML = '<div class="sb-hint sb-warn">预览命令没接上</div>'; return; }
    out.innerHTML = `<div class="sb-note"><pre data-preview>${escapeHtml(preview.requestSummary ?? preview.text ?? JSON.stringify(preview, null, 2))}</pre></div>`;
  });
}

// MARK: 高级设置（03-approach §3 的「高级设置 ›」入口）

/// 设置页的字段表：**显示名、说明、控件类型、取值范围**只有这一份。
///
/// 为什么不在模板里逐行写：这类页面的字段会随 Swift 侧增补而变长，
/// 而「区间」与「默认值」是**与 Rust `Settings::normalized()` 对齐**的——
/// 写错一个界不会有任何症状，只会让用户拖到一个本该被夹回去的值，
/// 然后怀疑是自己手滑了。集中一份之后，改一处就够。
const SETTING_FIELDS = [
  { group: '通用与外观', items: [
    { key: 'shell_mode', label: '界面形态', type: 'select',
      options: [['island', '灵动岛'], ['sidebar', '侧边栏']] },
    { key: 'appearance', label: '外观', type: 'select',
      options: [['system', '跟随系统'], ['light', '浅色'], ['dark', '深色']] },
    { key: 'dock_edge', label: '贴边位置', type: 'select',
      options: [['top', '上'], ['bottom', '下'], ['left', '左'], ['right', '右']] },
    { key: 'sidebar_edge', label: '侧边栏位置', type: 'select', hint: '拖动调整宽度，自动记忆',
      options: [['left', '左边'], ['right', '右边']] },
    { key: 'notification_policy', label: '通知策略', type: 'select',
      options: [['standard', '标准'], ['focus', '专注免打扰'], ['silent', '完全静默']] },
  ]},
  { group: '引擎与性能', items: [
    { key: 'sample_interval', label: '活动采样间隔', type: 'number', unit: '秒', min: 0.5, max: 600, step: 0.5 },
    { key: 'idle_sample_interval', label: '闲置采样间隔', type: 'number', unit: '秒', min: 0.5, max: 600, step: 0.5,
      hint: '至少与活动采样间隔相同' },
    { key: 'working_window', label: '工作判定窗口', type: 'number', unit: '秒', min: 10, max: 300, step: 5,
      hint: '窗口内有文件写入，视为工作中' },
    { key: 'min_working_hold', label: '工作状态保持', type: 'number', unit: '秒', min: 1, max: 300, step: 1,
      hint: '工作信号消失后延续此时长' },
    { key: 'active_session_window', label: '活跃会话窗口', type: 'number', unit: '秒', min: 60, max: 3600, step: 60 },
    { key: 'cpu_threshold', label: 'CPU 工作阈值', type: 'number', unit: '%', min: 1, max: 50, step: 1 },
    { key: 'collapse_delay', label: '自动收起延迟', type: 'number', unit: '秒', min: 0.2, max: 5, step: 0.1 },
    { key: 'battery_saver_enabled', label: '电池供电时降频', type: 'bool' },
    { key: 'runaway_cpu_alert', label: '持续高负载告警', type: 'bool', hint: '仅控制提醒，保留健康度判定' },
    { key: 'runaway_cpu_threshold', label: '高负载阈值', type: 'number', unit: '%', min: 10, max: 100, step: 1 },
    { key: 'runaway_duration_threshold', label: '高负载持续时间', type: 'number', unit: '秒', min: 30, max: 3600, step: 30 },
  ]},
  { group: 'Token 与预算', items: [
    { key: 'token_alert_enabled', label: 'Token 暴涨告警', type: 'bool' },
    { key: 'token_alert_threshold', label: '暴涨阈值', type: 'number', unit: 'token/分', min: 1000, max: 10000000, step: 1000 },
    { key: 'daily_token_budget', label: '24h Token 预算', type: 'number', unit: 'token', min: 0, max: 1000000000, step: 100000,
      hint: '滚动 24 小时统计；0 表示未设置' },
    { key: 'budget_alert_enabled', label: '预算告警', type: 'bool', hint: '仅控制提醒，保留用量统计' },
  ]},
  { group: '提醒', items: [
    { key: 'play_completion_sound', label: '完成提示音', type: 'bool' },
    { key: 'auto_anomalies_alert', label: '异常驻留告警', type: 'bool', hint: '仅控制提醒，保留卡死与健康度判定' },
  ]},
  { group: '界面与系统', items: [
    { key: 'compact_view', label: '紧凑视图', type: 'bool' },
    { key: 'hide_docked_sliver', label: '收起后隐藏贴条', type: 'bool' },
    { key: 'global_hot_key_enabled', label: '全局热键', type: 'bool',
      hint: 'Cmd/Ctrl+Shift+I' },
    { key: 'launch_at_login', label: '开机自启', type: 'bool' },
    { key: 'menu_bar_badge_mode', label: '菜单栏徽标', type: 'select',
      options: [['iconOnly', '仅图标'], ['activeCount', '活跃任务数'], ['tokenUsage', '今日 Token']] },
    { key: 'screen_follow_mode', label: '屏幕跟随', type: 'select',
      options: [['followMouse', '跟随鼠标所在屏'], ['mainScreen', '固定主屏'], ['builtInScreen', '优先内置屏'], ['externalScreen', '外接屏']] },
  ]},
];

/// 设置页外壳。先渲染出**当前值**（读的是后端 `state.settings`），
/// 不用占位符——设置页显示 0 再被真实值替换，用户会以为它闪了一下。
export function pageSettings() {
  const st = getState();
  const s = st.settings ?? {};
  const fields = new Map(SETTING_FIELDS.flatMap(group => group.items.map(field => [field.key, field])));
  const common = [
    { title: '界面', keys: ['appearance', 'dock_edge'] },
    { title: '通知与系统', keys: ['notification_policy', 'play_completion_sound', 'global_hot_key_enabled', 'launch_at_login'] },
  ];
  const commonKeys = new Set(common.flatMap(group => group.keys));
  const groupHtml = (title, items) => `<section class="settings-section"><h2>${escapeHtml(title)}</h2>${items.map(field => settingRow(field, s[field.key])).join('')}</section>`;
  const advanced = SETTING_FIELDS.map(group => groupHtml(group.group, group.items.filter(field => !commonKeys.has(field.key)))).join('');
  return `<div class="sb-page settings-page" data-settings-root>
    ${common.map(group => groupHtml(group.title, group.keys.map(key => fields.get(key)))).join('')}
    <details class="settings-more" data-settings-more${st.settingsAdvancedOpen ? ' open' : ''}>
      <summary><span>更多设置</span><span class="settings-more-caption">布局、采样与预算</span>${ICONS.chevRight}</summary>
      <p class="settings-footnote">监控阈值下次采样生效，超出范围的数值自动调整。</p>
      ${advanced}
    </details>
  </div>`;
}

function settingRow(field, raw) {
  const id = escapeHtml(field.key);
  const hint = field.hint ? `<span class="sb-hint">${escapeHtml(field.hint)}</span>` : '';
  let control;
  if (field.type === 'bool') {
    control = `<input type="checkbox" data-set="${id}"${raw ? ' checked' : ''}>`;
  } else if (field.type === 'select') {
    const options = field.options
      .map(([v, label]) => `<option value="${escapeHtml(v)}"${v === raw ? ' selected' : ''}>${escapeHtml(label)}</option>`)
      .join('');
    control = `<select data-set="${id}">${options}</select>`;
  } else {
    // 读不到就留空而不是写 0：这一栏的含义是「未核实」，0 会被读成「用户设成 0」
    const value = (raw === undefined || raw === null) ? '' : String(raw);
    control = `<input type="number" data-set="${id}" value="${escapeHtml(value)}"
      min="${field.min}" max="${field.max}" step="${field.step}">${field.unit ? `<span class="sb-unit">${escapeHtml(field.unit)}</span>` : ''}`;
  }
  return `<label class="sb-set">
    <span class="sb-set-copy"><span class="sb-set-label">${escapeHtml(field.label)}</span>${hint}</span>
    <span class="sb-set-ctl">${control}</span>
  </label>`;
}

/// 绑定设置页的输入。**写盘失败要明说**：静默失败会让用户以为改掉了。
export function bindSettings() {
  const st = getState();
  document.querySelectorAll('[data-settings-more]').forEach(details => {
    details.addEventListener('toggle', () => { st.settingsAdvancedOpen = details.open; });
  });
  const root = document.querySelector('[data-settings-root]');
  if (!root) return;
  root.querySelectorAll('[data-set]').forEach((el) => {
    const handler = async () => {
      const key = el.dataset.set;
      const field = SETTING_FIELDS.flatMap((g) => g.items).find((f) => f.key === key);
      let value;
      if (field.type === 'bool') value = el.checked;
      else if (field.type === 'select') value = el.value;
      else {
        // 空值不写：用户清空输入框不等于「设成 0」
        if (el.value.trim() === '') return;
        value = Number(el.value);
        if (!Number.isFinite(value)) {
          el.closest('.sb-set')?.appendChild(notice('请输入数字'));
          return;
        }
        // 越界就地夹回并回填，别让界面显示一个不会被采用的数
        if (field.min !== undefined) value = Math.min(Math.max(value, field.min), field.max);
        if (field.type === 'number' && Number.isInteger(field.step) && !Number.isInteger(value)) {
          value = Math.round(value);
        }
        el.value = value;
      }
      st.settings = st.settings ?? {};
      const previousSettings = { ...st.settings };
      st.settings[key] = value;
      try {
        await saveSettings({ [key]: value });
        if (key === 'shell_mode') await invoke('set_shell_mode', { mode: String(value) });
        if (key === 'sidebar_edge') await invoke('set_sidebar_edge', { edge: String(value) });
        if (field.type === 'number') el.value = st.settings[key];
        // 形态 / 外观 / 紧凑 / 微细条 / 贴边：五件都走同一条布局通道
        if (key === 'appearance' || key === 'compact_view' || key === 'dock_edge') {
          applyLayout();
        }
        if (key === 'hide_docked_sliver' && isIsland()) renderSliver();
        // 开机自启：设置项已存，**系统注册**另走一条命令。
        // 两者的结果可能不一致（系统拒绝了、写权限没了），所以以命令的返回为准回写，
        // 不让界面上留一个「显示已开、其实没开」的开关。
        if (key === 'launch_at_login') {
          const actual = await invoke('set_launch_at_login', { enabled: !!value })
            .catch((e) => { el.closest('.sb-set')?.appendChild(notice(`系统拒绝：${e}`)); return null; });
          if (actual === null) return;
          el.checked = actual;
          if (!actual && value) {
            el.closest('.sb-set')?.appendChild(notice('开机自启未成功，请重试'));
          }
        }
        // 全局热键：注册由后端每 5 秒对齐一次，这里只需把组合键写在页面上，
        // 让用户知道按哪组键——**而不是**让用户自己猜。
        if (key === 'global_hot_key_enabled' && el.checked) {
          st.hotkeyAccel = st.hotkeyAccel ?? 'Cmd/Ctrl+Shift+I';
        }
      } catch (error) {
        st.settings = previousSettings;
        if (field.type === 'bool') el.checked = !!previousSettings[key];
        else el.value = previousSettings[key];
        el.closest('.sb-set')?.appendChild(notice(`保存失败：${error}`));
      }
    };
    el.addEventListener(el.tagName === 'SELECT' ? 'change' : el.type === 'checkbox' ? 'change' : 'change', handler);
  });
}

function notice(text) {
  const node = document.createElement('div');
  node.className = 'sb-hint sb-warn';
  node.textContent = text;
  return node;
}

export function pageSettingsHeaderLabel() {
  return '设置';
}


/// Each source retains its own error; unreadable data must not become an empty list.
export async function hydrateProvider({ canRender = () => true } = {}) {
  const root = document.querySelector('[data-provider-root]');
  if (!root) return;
  const workspace = root.closest('[data-model-workspace]');
  bindModelWorkspace(workspace, () => hydrateConnections(workspace.querySelector('[data-connections-root]')), () => {hydrateMcp(workspace.querySelector('[data-mcp-root]'));hydrateClaudePlan(workspace.querySelector('[data-claude-plan]'));}, () => hydratePrompts(workspace.querySelector('[data-prompts-root]')));
  const currentRequest = pageRequest(root);
  const status = await invoke('provider_status').catch(() => null);
  if (!currentRequest() || !canRender()) return false;
  if (!status) {
    root.innerHTML = '<div class="sb-empty" role="alert">配置读取失败。</div><button type="button" class="mini-btn" data-provider-retry>重试</button>';
    root.querySelector('[data-provider-retry]').onclick = hydrateProvider;
    const directory = workspace?.querySelector('[data-model-directory]');
    if (directory) directory.innerHTML = '<div class="sb-empty" role="alert">配置读取失败，无法生成目录。请回到工具配置重试。</div>';
    return;
  }
  const errors = [];
  const profiles = await invoke('provider_list_profiles').catch(() => {
    errors.push('档位清单读取失败，请重试。'); return null;
  });
  const backups = await invoke('provider_list_backups').catch(() => {
    errors.push('备份清单读取失败，请重试。'); return null;
  });
  if (!currentRequest() || !canRender()) return false;
  renderProviderPage(root, status, profiles ?? [], backups ?? [], errors, profiles !== null);
  root.dataset.pageReady = 'true';
  focusWorkspaceTarget();
  scheduleLayoutLog();
  return true;
}

function providerField(key, label, value = '') {
  return `<label class="sb-field"><span>${label}</span><input aria-label="${label}" data-field="${key}" value="${escapeHtml(value)}" autocomplete="off" /></label>`;
}

export function renderProviderPage(root, status, profiles, backups, errors = [], profilesAvailable = true) {
  const directory = root.closest('[data-model-workspace]')?.querySelector('[data-model-directory]');
  if (directory) {
    const kind=directory.querySelector('.model-directory')?.dataset.catalogKind;
    const query=directory.querySelector('[data-catalog-query]')?.value;
    directory.innerHTML = modelDirectoryHtml(status, profiles, profilesAvailable, kind, query);
    bindModelDirectory(directory);
  }
  const activeName = status.active_profile_id
    ? (profiles.find((profile) => profile.id === status.active_profile_id)?.name ?? status.active_profile_id)
    : null;
  getState().providerActiveName = activeName ?? '';
  const model = status.configured_model ?? '未指定';
  const provider = status.configured_provider ?? '默认接口';
  const activeLine = status.config_error
    ? `<div role="alert">${escapeHtml(status.config_error)}</div>`
    : `当前配置：<b>${escapeHtml(model)}</b> · ${escapeHtml(provider)}<div class="meta">${activeName ? `与档位「${escapeHtml(activeName)}」一致` : '无唯一匹配档位'}</div>`;
  const limitations = `<div class="sb-note" data-limitations>${escapeHtml(status.limitations ?? '')}</div>`;
  const summary = `<div class="sb-profile provider-current" data-status>${activeLine}<div class="meta">新会话或重启后使用此配置。</div></div>`;
  if (root.closest('.wb-overview')) {
    root.innerHTML = `${summary}${status.drifted ? '<div class="sb-note">配置已变化，请到档位页核对。</div>' : ''}
      <p class="wb-provider-count">${errors.length ? '档位或备份读取失败' : `${profiles.length} 个本机档位 · ${backups.length} 份备份`}</p>
      <button type="button" class="mini-btn" data-provider-open>管理档位</button>`;
    root.querySelector('[data-provider-open]').onclick = () => {
      getState().workbenchPage = 'provider'; getState().route = 'list'; renderWorkbench();
    };
    return;
  }
  const drift = status.drifted ? `<div class="sb-note provider-drift" role="status">
    <b>配置与上次应用记录不同</b><p>上次目标：${escapeHtml(status.last_applied?.model ?? '')} · ${escapeHtml(status.last_applied?.provider_id ?? '')}。可能来自其他工具或手动修改。</p>
    <div class="actions"><button type="button" class="mini-btn" data-reapply${status.revision && status.last_applied?.wire_api === 'responses' ? '' : ' disabled'}>重新应用上次配置</button>
    <button type="button" class="mini-btn" data-keep-current${status.revision ? '' : ' disabled'}>保留当前配置</button></div></div>` : '';
  const warnings = [...errors, ...(status.record_error ? [status.record_error] : [])]
    .map(text => `<div class="sb-note" role="alert">${escapeHtml(text)}</div>`).join('');
  const rows = profiles.length === 0
    ? `<div class="sb-empty">${errors.length ? '档位清单暂不可用。' : '暂无档位。保存当前配置或添加档位。'}</div>`
    : profiles.map((profile) => `<div class="sb-profile" data-profile-row="${escapeHtml(profile.id)}">
        <div class="name">${escapeHtml(profile.name)}${profile.id === status.active_profile_id ? '<span class="sb-tag">配置一致</span>' : ''}</div>
        <button type="button" class="provider-model" data-edit="${escapeHtml(profile.id)}" aria-label="编辑档位 ${escapeHtml(profile.name)} 的模型与接口">${escapeHtml(profile.model)} <span>编辑</span></button>
        <div class="meta">${escapeHtml(profile.provider_id)} · ${escapeHtml(profile.base_url)}</div>${profile.wire_api !== 'responses' ? '<p class="sb-note">旧 Chat 档位：需核对接口并改为 Responses。</p>' : ''}
        <div class="actions"><button type="button" class="mini-btn" data-switch="${escapeHtml(profile.id)}"${status.revision && profile.wire_api === 'responses' ? '' : ' disabled'}>预览并应用</button>
        <button type="button" class="mini-btn" data-delete="${escapeHtml(profile.id)}">删除</button></div></div>`).join('');
  const backupRows = backups.length === 0 ? '<div class="sb-empty">应用前自动备份。</div>'
    : backups.slice(0, 8).map((backup) => `<div class="sb-backup"><span class="name">${escapeHtml(backup.name)}</span>
        <button type="button" class="mini-btn" data-restore="${escapeHtml(backup.name)}"${status.revision ? '' : ' disabled'}>对比并还原</button></div>`).join('');
  root.innerHTML = `${summary}<div class="sb-toast" data-toast role="status" aria-live="polite" aria-atomic="true" tabindex="-1" hidden></div>${drift}${warnings}
    <div class="provider-toolbar"><button type="button" class="mini-btn" data-capture${status.current_draft ? '' : ' disabled'}>保存当前配置</button>
    <button type="button" class="mini-btn" data-new-profile>添加档位</button><button type="button" class="mini-btn" data-provider-refresh>刷新</button></div>
    ${status.capture_notice ? `<p class="sb-note">${escapeHtml(status.capture_notice)}</p>` : ''}
    <div class="sb-section">已保存档位</div>${rows}
    <details class="provider-editor" data-provider-editor><summary>档位编辑器</summary>
      <form class="sb-form" data-provider-form>
        <div class="sb-section" data-editor-title>添加档位</div>
        ${providerField('name', '档位名称')}${providerField('model', '模型')}
        <label class="sb-field"><span>接口来源</span><select aria-label="接口来源" data-provider-template><option value="">自定义接口</option><option value="@responses">Responses 协议模板</option>${profiles.map(profile => `<option value="${escapeHtml(profile.id)}">已保存：${escapeHtml(profile.name)}</option>`).join('')}</select></label>
        <details data-provider-advanced><summary>接口与高级字段</summary><div class="sb-form">
          ${providerField('id', '档位标识')}${providerField('provider_id', 'Provider 标识')}
          ${providerField('provider_name', 'Provider 名称')}${providerField('base_url', 'API 地址')}
          ${providerField('env_key', '密钥环境变量名')}
          <label class="sb-field"><span>API 协议</span><select aria-label="API 协议" data-field="wire_api"><option value="responses">Responses</option><option value="chat" disabled>Chat · 旧档位，需改为 Responses</option></select></label>
        </div></details>
        <p class="sb-note">保存后需单独应用。密钥使用环境变量，API 地址不能含认证信息。</p>
        <div class="actions"><button type="submit" class="mini-btn" data-save-profile>保存档位</button><button type="button" class="mini-btn" data-cancel-edit>取消编辑</button></div>
        <div data-editor-error role="alert"></div>
      </form>
    </details>
    <details class="provider-backups"><summary>备份与还原（${backups.length}）</summary>${backupRows}</details>
    <details class="provider-transfer"><summary>导入与导出</summary><p class="sb-note">仅支持 AgentIsland 档位文件。导出至下载目录，包含配置与环境变量名，不含密钥。</p><div class="actions"><button type="button" class="mini-btn" data-profile-export>导出档位</button><label class="mini-btn provider-file">选择文件并预览<input type="file" accept=".json,application/json" aria-label="选择档位文件并预览" data-profile-import /></label></div></details>
    ${isWorkbench ? '<div class="actions"><button type="button" class="mini-btn" data-open-extensions>管理 MCP 与 Skills</button></div>' : '<details class="provider-capabilities" data-capabilities><summary>本机 MCP 与 Skills</summary><div data-capability-list></div></details>'}
    ${limitations}<div class="sb-confirm" data-confirm role="dialog" aria-modal="true" aria-label="确认配置操作" tabindex="-1" hidden></div>`;
  bindProviderEvents(root, status, profiles);
}

function bindProviderEvents(root, status, profiles) {
  const confirmBox = root.querySelector('[data-confirm]');
  const editor = root.querySelector('[data-provider-editor]');
  const form = root.querySelector('[data-provider-form]');
  let editorHasDraft = false;
  form.addEventListener('input', () => { editorHasDraft = true; });
  form.addEventListener('change', () => { editorHasDraft = true; });
  form.addEventListener('reset', () => { editorHasDraft = false; });
  let returnFocus;
  const closeConfirm = () => {
    confirmBox.hidden = true;
    const navigation = root.closest('[data-model-workspace]')?.querySelector('.model-navigation');
    if (navigation) navigation.inert = false;
    for (const child of root.children) child.inert = false;
    returnFocus?.focus();
  };
  const askConfirm = (text, onConfirm, label = '确认') => {
    returnFocus = document.activeElement;
    confirmBox.hidden = false;
    const navigation = root.closest('[data-model-workspace]')?.querySelector('.model-navigation');
    if (navigation) navigation.inert = true;
    for (const child of root.children) child.inert = child !== confirmBox;
    confirmBox.innerHTML = `<div class="text">${text}</div><div class="actions"><button type="button" class="mini-btn" data-yes>${escapeHtml(label)}</button><button type="button" class="mini-btn" data-no>取消</button></div>`;
    confirmBox.querySelector('[data-yes]').onclick = async (event) => {
      event.currentTarget.disabled = true;
      closeConfirm();
      await onConfirm();
    };
    confirmBox.querySelector('[data-no]').onclick = closeConfirm;
    confirmBox.onkeydown = (event) => {
      if (event.key === 'Escape') closeConfirm();
      if (event.key === 'Tab') {
        event.preventDefault();
        const controls = [...confirmBox.querySelectorAll('button:not(:disabled)')];
        previewFocusTarget(controls, document.activeElement, event.shiftKey)?.focus();
      }
    };
    confirmBox.querySelector('[data-no]').focus();
    confirmBox.scrollIntoView({ block: 'nearest' });
  };
  const apply = async (command, args) => {
    const feedback = beginProviderFeedback(root);
    try {
      let failure = '';
      const context=command==='provider_apply_profile'?workspaceFlow.operationContext('profile',args.id):null;
      const applied = await workspaceFlow.execute('profile', args.id, verifyWorkspace, () => invoke(context?'workspace_apply_profile':command, { ...args, revision: status.revision, ...(context?{context}:{}) })).catch(error => { failure = String(error); return null; });
      const refreshed = feedback.current() && await hydrateProvider({ canRender: feedback.current });
      showProviderToast(root, failure || !applied ? `应用失败：${failure || '命令没有返回结果'}`
        : `${applied.record_warning || '配置已写入'}。备份：${applied.backup_name}。请开启新会话，必要时重启 Codex。${refreshed ? '' : '页面未刷新，请刷新核对。'}`, feedback);
    } finally { feedback.dispose(); }
  };
  const preview = (choice, callback) => askConfirm(
    `<b>Codex 配置目标</b><br/>模型：${escapeHtml(status.configured_model ?? '未指定')} → ${escapeHtml(choice.model)}<br/>
    Provider：${escapeHtml(status.configured_provider ?? '原生默认')} → ${escapeHtml(choice.provider_id)}<br/>
    API 地址：${escapeHtml(choice.base_url)}<br/>写入 ${escapeHtml(status.config_path ?? '')}，先自动备份。运行中的会话不会立即换模型。`, callback);
  root.querySelectorAll('[data-switch]').forEach(el => { el.onclick = () => {
    const choice = profiles.find(p => p.id === el.dataset.switch);
    if (choice) preview(choice, () => apply('provider_apply_profile', { id: choice.id, expectedProfile: choice }));
  }; });
  const workspace = root.closest('[data-model-workspace]');
  const directory = workspace?.querySelector('[data-model-directory]');
  if (directory) directory.onclick = event => {
    const button = event.target.closest('[data-model-profile],[data-catalog-edit]');
    if (!button || button.disabled || !confirmBox.hidden) return;
    const choice = profiles.find(profile => profile.id === (button.dataset.modelProfile ?? button.dataset.catalogEdit));
    if (!choice) return;
    if (editor.open || editorHasDraft) {
      const feedback = directory.querySelector('[data-model-feedback]');
      feedback.hidden = false;
      feedback.textContent = '请先在工具配置中保存或取消编辑。';
      return;
    }
    workspace.querySelector('[data-model-view="tools"]').click();
    root.querySelectorAll(button.dataset.catalogEdit ? '[data-edit]' : '[data-switch]').forEach(control => {
      if ((control.dataset.switch ?? control.dataset.edit) === choice.id) { control.focus(); control.click(); }
    });
  };
  root.querySelector('[data-reapply]')?.addEventListener('click', () => {
    if (status.last_applied) preview(status.last_applied, () => apply('provider_reapply', {}));
  });
  root.querySelector('[data-keep-current]')?.addEventListener('click', async () => {
    const error = await invoke('provider_keep_current', { revision: status.revision }).catch(() => '保留失败，请刷新后重试');
    await hydrateProvider();
    showProviderToast(root, error || '已保留当前配置，并解除上次应用记录；Codex 配置文件未改动。');
  });
  root.querySelector('[data-provider-refresh]').onclick = () => {
    if (editor.open) askConfirm('刷新会关闭编辑器并丢弃未保存内容。继续？', hydrateProvider);
    else hydrateProvider();
  };
  root.querySelectorAll('[data-restore]').forEach(el => { el.onclick = async () => {
    const name = el.dataset.restore;
    let failure;
    const diff = await invoke('provider_preview_backup', { name }).catch(() => { failure = '备份预览失败，请刷新后重试'; return null; });
    if (!diff) { showProviderToast(root, failure); return; }
    if (diff.writable !== true) { showProviderToast(root, '当前配置或备份含私有字段或未支持形式，保持只读；未创建备份或还原。'); return; }
    const labels = ['模型', 'Provider', 'API 地址', '密钥环境变量名', '协议'];
    const table = `<table class="provider-diff"><thead><tr><th>字段</th><th>当前</th><th>备份</th></tr></thead><tbody>${labels.map((label, i) => `<tr${diff.current[i] !== diff.backup[i] ? ' class="changed"' : ''}><th>${label}</th><td>${escapeHtml(diff.current[i])}</td><td>${escapeHtml(diff.backup[i])}</td></tr>`).join('')}</tbody></table>`;
    if (diff.identical) { showProviderToast(root, '备份与当前配置完全相同，无需还原。'); return; }
    askConfirm(`<b>还原 ${escapeHtml(name)}</b>${table}${diff.mcp_changes?.length ? `<p>MCP 变更</p><ul>${diff.mcp_changes.map(change => `<li>${escapeHtml(change)}</li>`).join('')}</ul>` : ''}${diff.skills_changes?.length ? `<p>Skills 变更</p><ul>${diff.skills_changes.map(change => `<li>${escapeHtml(change)}</li>`).join('')}</ul>` : ''}<p>将还原完整 config.toml（含 MCP、Skills 与其他设置），并先备份当前文件。预览不展示密钥或认证信息。</p>`, async () => {
      const feedback = beginProviderFeedback(root);
      try {
        let failure = '';
        const context=workspaceFlow.operationContext('profile',name,true);
        const restored = await workspaceFlow.execute('profile', name, verifyWorkspace, () => invoke(context?'workspace_restore_profile':'provider_restore_backup', { name, revision: diff.revision, backupRevision: diff.backup_revision, ...(context?{context}:{}) }), true).catch(() => { failure = '配置或备份已变化，或无法写入；请刷新核对'; return null; });
        const refreshed = feedback.current() && await hydrateProvider({ canRender: feedback.current });
        showProviderToast(root, !restored ? `还原未完成：${failure || '未收到结果'}` : `${restored.record_warning || '配置已还原'}。还原前备份：${restored.backup_name}。请开启新会话。${refreshed ? '' : '页面未刷新，请刷新核对。'}`, feedback);
      } finally { feedback.dispose(); }
    });
  }; });
  root.querySelector('[data-profile-export]').onclick = async () => {
    const path = await invoke('provider_export_file').catch(() => null);
    showProviderToast(root, path === null ? '导出失败，请检查档位清单与下载目录权限后重试。' : `已保存到 ${path}；仅包含配置字段与环境变量名。`);
  };
  root.querySelector('[data-profile-import]').onchange = async (event) => {
    const file = event.target.files?.[0]; event.target.value = '';
    if (!file) return;
    if (file.size > 65536) { showProviderToast(root, '文件不能超过 64 KB'); return; }
    const text = await file.text().catch(() => null);
    if (text === null) { showProviderToast(root, '文件不可读'); return; }
    const result = await invoke('provider_preview_import', { text }).catch(() => null);
    if (!result) { showProviderToast(root, '导入预览失败：仅支持版本 1 档位文件，拒绝额外字段、无效接口与重复 ID。'); return; }
    if (!result.added.length) { showProviderToast(root, `没有新档位；跳过 ${result.skipped.length} 个已有 ID。`); return; }
    askConfirm(`<b>将增加 ${result.added.length} 个档位</b><p>${result.added.map(p => escapeHtml(p.name)).join('、')}</p><p>跳过 ${result.skipped.length} 个已有 ID，保留本机档位。导入后不会应用到 Codex。</p>`, async () => {
      const count = await invoke('provider_import_bundle', { text, revision: result.revision }).catch(() => null);
      await hydrateProvider(); showProviderToast(root, count === null ? '导入失败，清单可能已变化；请重新预览。' : `已导入 ${count} 个档位。`);
    });
  };
  let inventoryLoaded = false;
  root.querySelector('[data-open-extensions]')?.addEventListener('click', () => workspace?.querySelector('[data-model-view="extensions"]')?.click());
  root.querySelector('[data-capabilities]')?.addEventListener('toggle', async (event) => {
    if (!event.target.open || inventoryLoaded) return;
    inventoryLoaded = true;
    const list = root.querySelector('[data-capability-list]'); list.textContent = '读取本机清单…';
    const result = await invoke('provider_capabilities').catch(() => null);
    if (!root.isConnected) return;
    if (!result) { list.textContent = '清单不可读；关闭后重新展开可重试。'; inventoryLoaded = false; return; }
    list.innerHTML = `<p class="sb-note">只读清单：Codex MCP，以及 Codex、Claude Code 与共享 Skills。加载状态需在客户端确认；不执行命令或连接服务。</p>${result.notices.map(n => `<p class="sb-note" role="alert">${escapeHtml(n)}</p>`).join('')}${result.items.length ? result.items.map(item => `<div class="sb-profile"><div class="name">${escapeHtml(item.name)} <span class="sb-tag">${escapeHtml(item.kind)}</span></div><div class="meta">${escapeHtml(item.target)} · ${escapeHtml(item.status)}<br/>${escapeHtml(item.source)}</div></div>`).join('') : '<p class="sb-empty">检查范围内未发现配置或技能目录。</p>'}`;
  });
  root.querySelectorAll('[data-delete]').forEach(el => { el.onclick = () => {
    const id = el.dataset.delete;
    askConfirm(`删除档位 ${escapeHtml(id)}？Codex 配置文件不会改动。`, async () => {
      const error = await invoke('provider_delete_profile', { id }).catch(e => String(e));
      await hydrateProvider(); showProviderToast(root, error ? `删除失败：${error}` : '档位已删除');
    });
  }; });
  const fillForm = (choice, editing = false) => {
    editorHasDraft = true;
    for (const control of form.querySelectorAll('[data-field]')) {
      const value = choice[control.dataset.field] ?? '';
      control.value = value; control.defaultValue = value;
      if (control.dataset.field === 'id') control.readOnly = editing;
    }
    const template = root.querySelector('[data-provider-template]');
    if (template) template.value = profiles.find(p => p.provider_id === choice.provider_id && p.base_url === choice.base_url && p.env_key === choice.env_key && p.wire_api === choice.wire_api)?.id ?? '';
    root.querySelector('[data-editor-error]').textContent = '';
    root.querySelector('[data-editor-title]').textContent = editing ? '编辑档位' : '保存为新档位';
    editor.open = true;
    root.querySelector('[data-provider-advanced]').open = !choice.base_url;
    form.querySelector('[data-field="name"]').focus();
    editor.scrollIntoView({ block: 'nearest' });
  };
  const uniqueId = () => {
    let index = 1; while (profiles.some(p => p.id === `profile-${index}`)) index++;
    return `profile-${index}`;
  };
  const start = (choice, editing = false) => {
    const open = () => fillForm(choice, editing);
    if (editor.open) askConfirm('替换编辑器中的内容？未保存的内容会丢失。', open); else open();
  };
  root.querySelector('[data-capture]').onclick = () => {
    if (status.current_draft) start({ ...status.current_draft, id: uniqueId(), name: '' });
  };
  root.querySelector('[data-new-profile]').onclick = () => start({ id: uniqueId(), provider_id: 'custom', provider_name: '自定义接口', wire_api: 'responses', env_key: 'CODEX_API_KEY' });
  root.querySelectorAll('[data-edit]').forEach(el => { el.onclick = () => {
    const choice = profiles.find(p => p.id === el.dataset.edit); if (choice) start(choice, true);
  }; });
  root.querySelector('[data-provider-template]')?.addEventListener('change', (event) => {
    const protocol = event.target.value === '@responses' ? 'responses' : null;
    const choice = protocol ? { provider_id: 'custom', provider_name: '自定义接口', base_url: '', env_key: 'CODEX_API_KEY', wire_api: protocol, model: '' } : profiles.find(p => p.id === event.target.value);
    if (!choice) { root.querySelector('[data-provider-advanced]').open = true; return; }
    for (const key of ['provider_id', 'provider_name', 'base_url', 'env_key', 'wire_api']) {
      form.querySelector(`[data-field="${key}"]`).value = choice[key];
    }
    if (protocol) root.querySelector('[data-provider-advanced]').open = true;
    const modelControl = form.querySelector('[data-field="model"]');
    if (!modelControl.value) modelControl.value = choice.model;
  });
  root.querySelector('[data-cancel-edit]').onclick = () => {
    const reset = () => { form.reset(); editor.open = false; const feedback=directory?.querySelector('[data-model-feedback]'); if(feedback){feedback.hidden=true;feedback.textContent='';} };
    askConfirm('关闭编辑器并丢弃未保存内容？', reset);
  };
  form.onsubmit = async (event) => {
    event.preventDefault();
    const profile = {};
    form.querySelectorAll('[data-field]').forEach(control => { profile[control.dataset.field] = control.value.trim(); });
    const button = root.querySelector('[data-save-profile]');
    button.disabled = true;
    try {
      await invoke('provider_save_profile', { profile });
      await hydrateProvider(); showProviderToast(root, '档位已保存，应用前可预览差异。');
    } catch (error) {
      root.querySelector('[data-editor-error]').textContent = `保存失败：${String(error)}。原有内容已保留。`;
      root.querySelector('[data-provider-advanced]').open = true;
    } finally { button.disabled = false; }
  };
}

function showProviderToast(root, text, operation) {
  showProviderFeedback(root, text, operation);
  // Config details belong in the UI, not the application log.
}

// MARK: - 待办页（Phase 3）

/// 待办页骨架。与档位页同一套路：先出骨架，再由 `hydrateTodo()` 填内容。
export function pageTodo() {
  return `
    <div class="sb-page" data-todo-root>
      <div class="sb-empty">加载中…</div>
    </div>`;
}

/// 读一次待办并重画页面。**只重画页面、不整页重画侧边栏**——
/// 整页重画会让光标从输入框里掉出去，也会把角标与列表的更新顺序搅在一起。
export async function hydrateTodo() {
  const root = document.querySelector('[data-todo-root]');
  if (!root) return;
  const currentRequest = pageRequest(root);
  const todo = await invoke('todos_list').catch(() => null);
  if (!currentRequest()) return;
  if (!todo) {
    root.innerHTML = '<div class="sb-empty">读不到待办清单</div>';
    return;
  }
  renderTodoPage(root, todo);
  root.dataset.pageReady = 'true';
  scheduleLayoutLog();
}

function renderTodoPage(root, todo) {
  const draft = root.querySelector('[data-todo-input]')?.value ?? '';
  const focusedToggle = root.contains(document.activeElement) ? document.activeElement?.dataset?.toggle : null;
  // 读坏过就**说出来**：清单看起来是空的，但用户的东西并没有被删掉（留档了）
  const broken = todo.broken_backup
    ? `<div class="sb-note">上次的清单读不出来，已留档为 <code>${escapeHtml(todo.broken_backup)}</code>。当前显示的是空清单。</div>`
    : '';

  const rows = todo.items.length === 0
    ? '<div class="sb-empty">还没有待办。在下面输入，回车即可加一条。</div>'
    : todo.items
        .map(
          (item) => `<div class="sb-todo${item.done ? ' is-done' : ''}" data-todo-row="${escapeHtml(item.id)}">
            <button type="button" class="box" role="checkbox" aria-checked="${item.done}" aria-label="${escapeHtml(item.text)}" data-toggle="${escapeHtml(item.id)}">${item.done ? '✓' : ''}</button>
            <span class="text">${escapeHtml(item.text)}</span>
            <button type="button" class="x" aria-label="删除待办：${escapeHtml(item.text)}" data-remove="${escapeHtml(item.id)}">×</button>
          </div>`,
        )
        .join('');

  const clearBtn = todo.items.some((item) => item.done)
    ? '<button type="button" class="mini-btn" data-clear-done>清除已完成</button>'
    : '';

  root.innerHTML = `
    ${broken}
    ${rows}
    <div class="sb-todo-add">
      <input aria-label="新增待办" data-todo-input placeholder="输入待办，回车添加" maxlength="500" />
    </div>
    <div class="sb-todo-foot">${clearBtn}</div>`;

  updateTodoBadge(root, todo.pending);

  // 回车加条：极简列表的全部交互就是这一下
  const input = root.querySelector('[data-todo-input]');
  if (input) {
    input.value = draft;
    input.onkeydown = async (event) => {
      if (event.key !== 'Enter' || event.isComposing) return;
      const text = input.value;
      if (!text.trim()) return;
      input.value = ''; // 先清空：命令慢的时候用户不会重复按回车加两条一样的
      const next = await invoke('todos_add', { text }).catch((error) => ({ failure: String(error) }));
      if (next?.failure) {
        showTodoToast(root, next.failure);
        input.value = text; // 失败就把内容还给用户，别让他重打
        return;
      }
      renderTodoPage(root, next);
      root.querySelector('[data-todo-input]')?.focus();
    };
    if (!focusedToggle && document.documentElement.classList.contains('shell-sidebar')) input.focus();
  }

  root.querySelectorAll('[data-toggle]').forEach((el) => {
    el.onclick = async () => {
      const next = await invoke('todos_toggle', { id: el.dataset.toggle }).catch((error) => ({ failure: String(error) }));
      if (next?.failure) showTodoToast(root, next.failure);
      else renderTodoPage(root, next);
    };
  });

  root.querySelectorAll('[data-remove]').forEach((el) => {
    el.onclick = async () => {
      const next = await invoke('todos_remove', { id: el.dataset.remove }).catch((error) => ({ failure: String(error) }));
      if (next?.failure) showTodoToast(root, next.failure);
      else renderTodoPage(root, next);
    };
  });

  if (focusedToggle) root.querySelector(`[data-toggle="${CSS.escape(focusedToggle)}"]`)?.focus({ preventScroll: true });
  const clear = root.querySelector('[data-clear-done]');
  if (clear) {
    // 「清除已完成」不弹确认：它只删已经勾掉的条目，而且删除本身就是用户点出来的
    clear.onclick = async () => {
      const next = await invoke('todos_clear_done').catch((error) => ({ failure: String(error) }));
      if (next?.failure) showTodoToast(root, next.failure);
      else renderTodoPage(root, next);
    };
  }
}

/// 更新侧栏角标**并记住计数**：下次整页重画时用它，不必再去读一次文件。
function updateTodoBadge(root, pending) {
  const st = getState();
  st.todosPending = pending;
  const badge = document.querySelector('[data-nav-count="todo"]');
  if (badge) {
    badge.textContent = String(pending);
    badge.hidden = pending === 0;
  }
  void root;
}

function showTodoToast(root, text) {
  const toast = document.createElement('div');
  toast.className = 'sb-toast';
  toast.textContent = text;
  root.appendChild(toast);
  // Toast 会随下一次重画消失，而**失败信息不该只存在一瞬间**：
  // 同时写进应用日志（`log_from_ui`），事后还能查。
  invoke('log_from_ui', { message: `[todo] ${text}` }).catch(() => {});
}

/// 下一帧再量：刚 `innerHTML` 完的那一帧尺寸可能还没稳定。
///
/// **按签名去抖**：`renderSidebar` 在监控页会随每一次引擎推送重跑（每 2 秒），
/// 不去抖就会把日志刷成一片。签名（页面 + 窗口尺寸 + 两列宽度）变了才记一行——
/// 于是「换个页面」或「窗口被拉到别的宽度」都会留证据，而重复渲染不会。
let lastLayoutSignature = '';
function scheduleLayoutLog() {
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      const signature = [
        getState().route ?? '',
        innerWidth,
        innerHeight,
        Math.round(document.querySelector('.sb-nav')?.getBoundingClientRect().width ?? 0),
        Math.round(document.querySelector('.sb-main')?.getBoundingClientRect().width ?? 0),
      ].join('|');
      if (signature === lastLayoutSignature) return;
      lastLayoutSignature = signature;
      logSidebarLayout();
    }),
  );
}

/// 把侧边栏的布局尺寸写进应用日志（每次首屏一条）
export function logSidebarLayout() {
  try {
    const rect = (selector) => {
      const el = document.querySelector(selector);
      if (!el) return `${selector}=缺失`;
      const box = el.getBoundingClientRect();
      return `${selector}=${Math.round(box.width)}x${Math.round(box.height)}`;
    };
    const rootStyle = getComputedStyle(document.getElementById('root'));
    // 表单控件也要量：溢出这类失效的根因常常是某个控件的固有最小宽度
    // 把整列撑开（`min-width: auto` 是 flex/grid 子项的默认值），而不是列本身有问题。
    const widest = [...document.querySelectorAll('.sb-body input, .sb-body select')]
      .map((el) => Math.round(el.getBoundingClientRect().width))
      .sort((a, b) => b - a)[0];
    invoke('log_from_ui', {
      message:
        `sb-layout win=${innerWidth}x${innerHeight} root.display=${rootStyle.display} ` +
        `${rect('.sb')} ${rect('.sb-nav')} ${rect('.sb-main')} ${rect('.sb-body')} ` +
        `widestInput=${widest ?? '无'} bodyScrollW=${document.querySelector('.sb-body')?.scrollWidth ?? '?'}`,
    }).catch(() => {});
  } catch (error) {
    // 诊断不该把界面弄坏
  }
}
