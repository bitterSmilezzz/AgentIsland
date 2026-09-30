// 灵动岛视图渲染（IslandView / AgentRowView / TokenSummaryBar / SubViews 的 Web 对应物）
import { invoke } from './tauri.js';
import { isIsland, isWorkbench } from './shell.js';
import { getState, setState, expand, collapse, armCollapseTimer, scheduleRender, resizeToContent, applyAppearance, applyEdge, applyLayout } from './main.js';

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

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

/// 横幅标题：类型前缀只在消息本身没有带上时才加（防「需要确认: 需要确认: …」）
function bannerTitle(ev) {
  const prefix = ev.event_type === 'completed' ? '已完成' : ev.event_type === 'costSpike' ? '告警' : '需要确认';
  const msg = eventSummary(ev);
  return msg.startsWith('已完成') || msg.startsWith('需要确认') || msg.startsWith('告警') ? msg : `${prefix}: ${msg}`;
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
    <div class="glyph" style="font-size:${size >= 34 ? 17 : 13.5}px">${esc(snap.glyph)}</div></div>`;
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
  chevLeft: '<svg viewBox="0 0 16 16"><path d="M10.5 8 5.5 13 4 11.6 7.6 8 4 4.4 5.5 3z"/></svg>',
  chevRight: '<svg viewBox="0 0 16 16"><path d="M5.5 8 10.5 3 12 4.4 8.4 8 12 11.6 10.5 13z"/></svg>',
  close: '<svg viewBox="0 0 16 16"><path d="M4.4 3 8 6.6 11.6 3 13 4.4 9.4 8l3.6 3.6-1.4 1.4L8 9.4 4.4 13 3 11.6 6.6 8 3 4.4z"/></svg>',
  hand: '<svg viewBox="0 0 16 16"><path d="M7 2v6H6V3.5a.75.75 0 0 0-1.5 0V10l-1-1.3a1.1 1.1 0 0 0-1.7 1.4l3.2 4A3 3 0 0 0 7.4 15H9a4 4 0 0 0 4-4V6.5a.75.75 0 0 0-1.5 0V9h-.5V4.5a.75.75 0 0 0-1.5 0V9H9V2.75A.75.75 0 0 0 8.25 2 1.25 1.25 0 0 0 7 3.25z"/></svg>',
  terminal: '<svg viewBox="0 0 16 16"><path d="M2 3h12v10H2V3zm1.5 1.5v7h9v-7h-9zM4.8 6l1.8 2-1.8 2 1 1 2.6-3L5.8 5l-1 1z"/></svg>',
  jump: '<svg viewBox="0 0 16 16"><path d="M4 3h4v1.5H5.5v6h6V8H13v4H3V3h1z"/><path d="M9 3h4v4h-1.5V5.6L8 9.1 7 8.1l3.5-3.6H9V3z"/></svg>',
  warn: '<svg viewBox="0 0 16 16"><path d="M8 1.5 15 14H1L8 1.5zM7.3 6v4h1.4V6H7.3zm.7 7a.9.9 0 1 0 0-1.8.9.9 0 0 0 0 1.8z"/></svg>',
};

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
  const alert = !!eng?.has_attention || eng?.latest_event?.event_type === 'costSpike';

  const edge = st.settings?.dock_edge ?? 'top';
  const hitClass = edge === 'top' ? 'hit-top' : edge === 'bottom' ? 'hit-bottom'
    : edge === 'left' ? 'hit-left' : 'hit-right';
  const vertical = edge === 'left' || edge === 'right';

  // 「收起时隐藏微细条」：不画那条可见的细条，但**保留热区**。
  //
  // 为什么保留：微细条是收起态唯一的唤回入口（`mouseenter` 即展开）。
  // 连热区一起去掉的话，窗口会变成一块看不见也点不到的死区——
  // 用户只能靠托盘或快捷键把它叫出来，而那两样在 macOS 侧还要用户自己知道。
  // 所以「隐藏」隐藏的是**胶囊把手**，不是交互面积。
  const hidden = st.settings?.hide_docked_sliver === true;
  const status = alert ? '有提醒' : working ? '工作中' : '待机';
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
    const base = ev.externally_delivered ? (isCost ? '外部告警' : '外部确认') : isCost ? '告警' : '待确认';
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
  const active = visible.find((s) => s.level === 'working' && s.current_action);
  if (active) {
    const others = visible.filter((s) => s.level === 'working').length - 1;
    return { title: active.name, subtitle: active.current_action, badge: others > 0 ? `+${others}` : '工作中', tint: 'var(--working)', icon: ICONS.terminal };
  }
  const workingCount = visible.filter((s) => s.level === 'working').length;
  const completedCount = visible.filter((s) => s.level === 'completed').length;
  const uncertainCount = visible.filter((s) => s.level === 'idle' && s.observability?.code && s.observability.code !== 'observed').length;
  const text = workingCount > 0 ? `${workingCount} 个 Agent 正在工作`
    : completedCount > 0 ? `${completedCount} 个任务已完成`
      : uncertainCount > 0 ? `${uncertainCount} 个 Agent 状态待核实`
        : visible.length === 0 ? '暂无运行中的 Agent' : '全部 Agent 待机';
  return { title: text, subtitle: null, badge: null, tint: workingCount > 0 ? 'var(--working)' : uncertainCount > 0 ? 'var(--warning)' : 'var(--idle)', icon: null };
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
  return `<span class="health-chip" data-grade="${esc(health.grade)}" title="${esc(tips)}">${esc(health.score)}</span>`;
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
    tokensText: snap.token_usage && snap.token_usage.tokens24h > 0 ? compact(snap.token_usage.tokens24h) : '—',
    /** 只有工作/等待确认才有「当前动作」，其余形态是空的 */
    actionText: hasAction ? (snap.current_action ?? '') : '',
    hasAction: hasAction && !!snap.current_action,
    activityText: snap.last_activity_text ?? '',
    isAttention: snap.level === 'attention',
  };
}

function rowHtml(snap) {
  const model = agentRowModel(snap);
  return `
    <button type="button" class="row" data-agent="${esc(model.id)}" aria-label="${esc(model.name)}：${esc(model.statusText)}，查看详情">
      <div class="row-line1">
        <span class="island-agent-icon">${navigationIcon('terminal')}</span>
        <div class="row-name-col">
          <div class="row-name">${esc(model.name)}</div>
          <div class="row-sub"><i class="island-state-dot" style="background:${model.statusColor}"></i><span>${esc(model.statusText)}</span></div>
        </div>
        <div class="island-row-usage"><strong>${model.tokensText}</strong><span>24h tokens</span></div>
        ${healthChip(snap)}
      </div>
      ${model.hasAction ? `<div class="action-bar" title="${esc(model.actionText)}">${esc(model.actionText)}</div>` : model.activityText ? `<div class="island-row-activity">${esc(model.activityText)}</div>` : ''}
    </button>`;
}

// MARK: 主卡（IslandView.expandedCard）

export function renderCard() {
  const st = getState();
  const eng = st.engine ?? {
    snapshots: [], grand_total: { tokens24h: 0, tokens_total: 0, cost24h: 0, cost_total: 0 },
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
  } else if (st.route.startsWith('agentDetail:')) {
    content = pageAgentDetail(eng, st.route.split(':')[1]);
  } else {
    content = listCard(eng, st, visible, dark, edge);
  }

  root.innerHTML = `
    <div class="card dock-${edge}">
      <div class="card-inner">${content}</div>
    </div>`;

  bindCardEvents(eng, st);
}

function listCard(eng, st, visible, dark, edge) {
  const hp = headerPresentation(eng);
  const gt = eng.grand_total ?? {};
  const hasSummary = (gt.tokens24h ?? 0) > 0 || (gt.tokens_total ?? 0) > 0;
  const ev = eng.latest_event;

  const chev = edge === 'top' ? ICONS.chevUp : edge === 'bottom' ? ICONS.chevDown : edge === 'left' ? ICONS.chevLeft : ICONS.chevRight;
  const themeIcon = dark ? ICONS.sun : ICONS.moon;

  const banner = ev ? `
    <div class="divider"></div>
    <div class="banner" style="background:color-mix(in srgb, ${ev.event_type === 'costSpike' ? 'var(--danger)' : 'var(--warning)'} 10%, transparent)">
      <div class="line1">
        <span class="icon" style="color:${ev.event_type === 'costSpike' ? 'var(--danger)' : 'var(--warning)'}">${ev.event_type === 'completed' ? '\uE73E' : '\uE7BA'}</span>
        <span class="title" style="color:${ev.event_type === 'costSpike' ? 'var(--danger)' : 'var(--warning)'}">${esc(bannerTitle(ev))}</span>
        <button type="button" class="mini-btn" data-banner-detail>${st.lastEventDetail ? '原因 ˄' : '原因 ˅'}</button>
        <button type="button" class="mini-btn" data-banner-close style="padding:2px 5px">${ICONS.close}</button>
      </div>
      ${st.lastEventDetail && ev.detail ? `<div class="detail">${esc(ev.detail)}</div>` : ''}
      <div class="actions"><span class="mini-btn jump" data-agent-jump="${esc(ev.agent_id)}">${ICONS.jump}直达</span></div>
    </div>` : '';

  const search = st.searchActive ? `
    <div class="searchbar">
      ${ICONS.search.replace('<svg', '<svg width="10" height="10" style="color:var(--cyan)"')}
      <input id="searchInput" placeholder="按名称或 CLI 快速过滤..." value="${esc(st.searchText ?? '')}" />
    </div>` : '';

  const filtered = st.searchActive && st.searchText
    ? visible.filter((s) => s.name.toLowerCase().includes(st.searchText.toLowerCase()) || s.id.includes(st.searchText.toLowerCase()))
    : visible;

  const list = filtered.length === 0
    ? `<div class="empty">${st.searchActive ? `${navigationIcon('search')}<span>未找到匹配「${esc(st.searchText)}」的智能体</span>`
      : `${navigationIcon('terminal')}<span>没有活跃的 Agent</span>`}</div>`
    : `<div class="list">${filtered.map(rowHtml).join('')}</div>`;

  const summary = hasSummary ? `
    <button type="button" class="summary" data-analytics aria-label="查看用量分析">
      <span class="island-summary-metric"><span>最近 24 小时</span><strong>${compact(gt.tokens24h)}</strong><small>${costText(gt.cost24h, gt.cost_estimated) || 'tokens'}</small></span>
      <span class="island-summary-metric"><span>累计用量</span><strong>${compact(gt.tokens_total)}</strong><small>${costText(gt.cost_total, gt.cost_estimated) || 'tokens'}</small></span>
      <span class="island-summary-link">用量分析 ${navigationIcon('chart')}</span>
    </button>` : '';

  const statusColor = eng.has_attention ? 'var(--warning)' : eng.any_working ? 'var(--working)' : 'var(--idle)';

  return `
    <div class="header" data-drag>
      <div class="status-dot" style="background:${statusColor}"></div>
      <div class="header-titles">
        <div class="header-line1">
          <span class="header-title">${esc(hp.title)}</span>
          ${eng.demo ? '<span class="badge" style="color:var(--cyan);background:color-mix(in srgb, var(--cyan) 14%, transparent);border:0.5px solid color-mix(in srgb, var(--cyan) 35%, transparent)">演示数据</span>' : ''}
          ${hp.badge ? `<span class="badge" style="color:${hp.tint};background:color-mix(in srgb, ${hp.tint} 14%, transparent);border:0.5px solid color-mix(in srgb, ${hp.tint} 35%, transparent)">${esc(hp.badge)}</span>` : ''}
        </div>
        ${hp.subtitle ? `<div class="header-line2">
          <span style="color:${hp.tint};font-family:var(--font-icon)">${hp.iconChar ?? ''}</span>
          <span class="sub" style="color:${hp.tint}">${esc(hp.subtitle)}</span></div>` : ''}
      </div>
      <div class="header-icons">
        <span class="header-count">本机 · ${visible.length} 在线</span>
        <button type="button" class="icon-btn" data-search title="即时搜索过滤 (/)" aria-label="搜索智能体">${ICONS.search}</button>
        <button type="button" class="icon-btn" data-theme title="外观主题" aria-label="切换外观">${themeIcon}</button>
        <button type="button" class="icon-btn" data-analytics title="Token 用量分析" aria-label="用量分析">${ICONS.wrench}</button>
        <button type="button" class="icon-btn" data-collapse title="收起灵动岛" aria-label="收起灵动岛">${chev}</button>
      </div>
    </div>
    <div class="divider"></div>
    ${banner}
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

function renderReportBody(report) {
  const u = report.usage;
  const now = new Date();
  const remaining = Math.max(1, new Date(now.getFullYear(), now.getMonth() + 1, 0).getDate() - now.getDate());
  const hourly = report.hourly30d;

  // 趋势（近 24h）
  const pts = [];
  let max = 1;
  const cutoff = Date.now() - 24 * 3600 * 1000;
  for (const [ts, v] of hourly) if (ts >= cutoff) { pts.push([ts, v]); if (v > max) max = v; }
  const W = 270, H = 96;
  const xy = pts.map(([ts, v], i) => [4 + (pts.length === 1 ? W / 2 - 4 : i / (pts.length - 1) * (W - 8)), H - 16 - v / max * (H - 30)]);
  const line = xy.map((p, i) => `${i === 0 ? 'M' : 'L'}${p[0].toFixed(1)},${p[1].toFixed(1)}`).join(' ');
  const area = `${line} L${xy.length ? xy[xy.length - 1][0].toFixed(1) : W - 4},${H - 14} L${xy.length ? xy[0][0].toFixed(1) : 4},${H - 14} Z`;
  const peakIdx = pts.reduce((bi, _, i) => (pts[i][1] > pts[bi][1] ? i : bi), 0);

  // 热力图（近 24 格）
  const heat = [];
  for (let i = 23; i >= 0; i--) {
    const bucket = (Date.now() - i * 3600 * 1000);
    const ts = bucket - bucket % 3600000;
    const v = hourly.find(([t]) => t === ts)?.[1] ?? 0;
    const alpha = v <= 0 ? 0.08 : Math.max(0.15, Math.sqrt(v / Math.max(1, max)) * 0.9);
    heat.push(`<i style="background:color-mix(in srgb, var(--cyan) ${Math.round(alpha * 100)}%, transparent)"></i>`);
  }

  const maxTool = Math.max(1, ...report.models24h.map((m) => m.tokens));
  const t24 = report.models24h.reduce((a, m) => a + m.tokens, 0);

  return `
    <div class="segmented" data-seg>
      <div class="on">24h</div><div data-range="7">7天</div><div data-range="30">30天</div>
    </div>
    <div class="card-box usage-forecast">
      <h4>月末用量与成本预测</h4>
      <div class="totals" style="margin-top:8px;justify-content:flex-start;gap:14px">
        <div><div class="big-num" style="font-size:14px">${compact(u.tokens24h * remaining)}</div><div class="num-label">预估月末消耗</div></div>
        <div><div class="big-num c-working" style="font-size:14px">${costText(u.cost24h, u.cost_estimated) ? `~$${(u.cost24h * remaining).toFixed(2)}` : '—'}</div><div class="num-label">预估月末费用</div></div>
        <div><div class="big-num" style="font-size:14px">${remaining} 天</div><div class="num-label">当月剩余自然日</div></div>
      </div>
    </div>
    <div class="card-box usage-totals">
      <div class="totals">
        <div><div class="big-num">${compact(u.tokens24h)}</div><div class="num-label">24h 用量</div></div>
        <div><div class="big-num">${costText(u.cost24h, u.cost_estimated) || '—'}</div><div class="num-label">费用</div></div>
        <div><div class="big-num">${compact(u.tokens_total)}</div><div class="num-label">累计</div></div>
      </div>
    </div>
    <div class="card-box usage-trend">
      <div style="display:flex;align-items:center"><h4>使用趋势</h4>
        <span style="margin-left:auto;font-size:9.5px;color:var(--cyan);font-family:var(--font-mono)">峰值 ${compact(max)}</span></div>
      <svg class="usage-trend" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="最近 24 小时用量趋势" style="margin-top:6px">
        ${[1, 2, 3].map((i) => `<line x1="4" x2="${W - 4}" y1="${(H - 16) * i / 4}" y2="${(H - 16) * i / 4}" stroke="var(--hairline)" stroke-width="0.5"/>`).join('')}
        ${xy.length > 1 ? `
          <path d="${area}" fill="color-mix(in srgb, var(--cyan) 14%, transparent)"/>
          <path d="${line}" fill="none" stroke="var(--cyan)" stroke-width="1.4"/>
          <circle cx="${xy[peakIdx][0]}" cy="${xy[peakIdx][1]}" r="3" fill="var(--cyan)" stroke="#fff" stroke-width="1"/>` : ''}
      </svg>
      <div style="display:flex;justify-content:space-between;font-size:8px;color:var(--text-faint)">
        <span>${pts.length ? new Date(pts[0][0]).toTimeString().slice(0, 5) : ''}</span>
        <span>${pts.length ? new Date(pts[pts.length - 1][0]).toTimeString().slice(0, 5) : ''}</span>
      </div>
    </div>
    <div class="card-box usage-rhythm">
      <div style="display:flex;align-items:center"><span style="font-size:10.5px;color:var(--text-faint)">24h 协同节律</span>
        <span style="margin-left:auto;font-size:9.5px;color:var(--text)">活跃 ${heat.filter((h) => !h.includes('8%')).length}/24h</span></div>
      <div class="heat">${heat.join('')}</div>
    </div>
    <div class="card-box usage-models">
      <h4>按工具用量</h4>
      ${report.models24h.map((m) => `
        <div class="model-row">
          <div class="top"><span>${esc(m.model)}</span>
            <span class="r"><span class="tk">${compact(m.tokens)}</span><span class="cost">${costText(m.cost, m.cost_estimated)}</span></span></div>
          <div class="hbar"><i style="width:${Math.max(2, 100 * m.tokens / maxTool)}%"></i></div>
        </div>`).join('') || '<div class="c-faint" style="font-size:10px;margin-top:6px">暂无按工具明细</div>'}
    </div>`;
}

// MARK: Agent 详情页（AgentDetailView）

export function pageAgentDetail(eng, agentId) {
  const snap = eng.snapshots.find((s) => s.id === agentId);
  const name = snap?.name ?? agentId;
  return `
    <div class="page" data-page="agentDetail" data-agent-id="${esc(agentId)}">
      <div class="page-header">
        <button type="button" class="back-btn" aria-label="返回监控" data-back>‹</button>
        <div class="page-titles">
          <div class="t">${esc(name)}</div>
          <div class="s">Agent 详情 · 双口径总览 + 按模型拆分</div>
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
        <div><div class="big-num" style="font-size:14px">${costText(u.cost24h, u.cost_estimated) || '—'}</div><div class="num-label">24h 花费</div></div>
        <div><div class="big-num" style="font-size:14px">${compact(u.tokens_total)}</div><div class="num-label">累计</div></div>
        <div><div class="big-num" style="font-size:14px">${costText(u.cost_total, u.cost_estimated) || '—'}</div><div class="num-label">累计花费</div></div>
      </div>
    </div>
    <div class="card-box">
      <h4>按模型拆分（24h）</h4>
      ${report.models24h.map((m) => {
    const maxT = Math.max(1, ...report.models24h.map((x) => x.tokens));
    return `<div class="model-row" data-model="${esc(m.model)}">
          <div class="top"><span>${esc(m.model)}</span>
            <span class="r"><span class="tk">${compact(m.tokens)}</span><span class="cost">${costText(m.cost, m.cost_estimated)}</span></span></div>
          <div class="hbar"><i style="width:${Math.max(2, 100 * m.tokens / maxT)}%"></i></div>
        </div>`;
  }).join('') || `
        <div class="c-faint" style="font-size:11px;margin-top:6px">未发现本地明细</div>
        <div class="c-faint" style="font-size:9.5px;margin-top:4px">该档案登记的明细源本轮没有可读取的用量记录；「没取到」不等于「真的零用量」。</div>`}
    </div>`;
}

// MARK: 卡片事件绑定

function bindCardEvents(eng, st) {
  const root = document.getElementById('root');

  // 顶栏拖拽 + 松手吸附
  const header = root.querySelector('[data-drag]');
  if (header) {
    header.setAttribute('data-tauri-drag-region', '');
    header.addEventListener('mouseup', async () => {
      // 原生拖拽结束后吸附最近边
      setTimeout(async () => {
        const edge = await invoke('snap_nearest_edge', { width: 330, height: root.querySelector('.card').getBoundingClientRect().height + 20 });
        st.settings.dock_edge = edge;
        applyEdge();
        renderCard();
      }, 60);
    });
  }

  root.querySelectorAll('[data-back]').forEach((el) => el.addEventListener('click', () => {
    if (st.route.startsWith('agentDetail:')) { st.route = 'list'; }
    else { st.route = 'list'; }
    renderCard();
  }));

  const searchBtn = root.querySelector('[data-search]');
  if (searchBtn) searchBtn.addEventListener('click', () => {
    st.searchActive = !st.searchActive;
    if (!st.searchActive) st.searchText = '';
    renderCard();
  });

  const themeBtn = root.querySelector('[data-theme]');
  if (themeBtn) themeBtn.addEventListener('click', () => {
    const dark = document.documentElement.classList.contains('theme-dark');
    st.settings.appearance = dark ? 'light' : 'dark';
    applyAppearance(st.settings.appearance);
    invoke('save_settings', { newSettings: st.settings }).catch(() => {});
    renderCard();
  });

  root.querySelectorAll('[data-analytics]').forEach((el) => el.addEventListener('click', () => {
    st.route = 'tokenAnalytics';
    renderCard();
  }));

  const collapseBtn = root.querySelector('[data-collapse]');
  if (collapseBtn) collapseBtn.addEventListener('click', collapse);

  root.querySelectorAll('[data-agent]').forEach((el) => el.addEventListener('click', () => {
    st.route = `agentDetail:${el.dataset.agent}`;
    renderCard();
    hydrateReport();
  }));

  const jump = root.querySelector('[data-agent-jump]');
  if (jump) jump.addEventListener('click', (e) => {
    e.stopPropagation();
    // 直达窗口：Windows 端后续接 Win32 激活；v1 回到详情页
    st.route = `agentDetail:${jump.dataset.agentJump}`;
    renderCard();
    hydrateReport();
  });

  const bannerDetail = root.querySelector('[data-banner-detail]');
  if (bannerDetail) bannerDetail.addEventListener('click', (e) => {
    e.stopPropagation();
    st.lastEventDetail = !st.lastEventDetail;
    renderCard();
  });

  const bannerClose = root.querySelector('[data-banner-close]');
  if (bannerClose) bannerClose.addEventListener('click', async (e) => {
    e.stopPropagation();
    bannerClose.closest('.banner').style.display = 'none';
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

  root.addEventListener('mouseleave', () => { if (st.expanded) armCollapseTimer(); });
  root.addEventListener('mouseenter', () => clearTimeout(st.collapseTimer));

  // 自愈高度：横幅/字体/异步内容造成的滞后由周期校正兜底
  if (!globalThis.__heightHealer) {
    globalThis.__heightHealer = setInterval(() => {
      const st2 = getState();
      if (st2.expanded) resizeToContent();
    }, 800);
  }

  // 分析页/详情页数据加载
  if (st.route === 'tokenAnalytics' || st.route.startsWith('agentDetail:')) hydrateReport();
}

function bindRowClicks(st) {
  document.querySelectorAll('[data-agent]').forEach((el) => {
    el.onclick = () => {
      st.route = `agentDetail:${el.dataset.agent}`;
      renderCard();
      hydrateReport();
    };
  });
}

export async function hydrateReport() {
  const st = getState();
  const bodies = [...document.querySelectorAll('[data-report-root]')];
  await Promise.all(bodies.map(async (body) => {
    const page = body.closest('[data-page]');
    const analytics = page?.dataset.page === 'tokenAnalytics';
    // 空 ID 是后端约定的全部启用档案汇总，不取第一个在线工具。
    const agentId = analytics ? '' : page?.dataset.agentId ?? '';
    const report = await invoke('get_report', { agentId }).catch(() => null);
    // 导航可能已经换页；过期响应不写入新页面。
    if (!body.isConnected) return;
    if (!report) {
      body.innerHTML = `<div class="report-empty">${navigationIcon('chart')}<span>暂无本地明细数据</span></div>`;
      return;
    }
    const snap = st.engine?.snapshots.find((entry) => entry.id === agentId);
    body.innerHTML = analytics ? renderReportBody(report) : renderDetailBody(report, snap);
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
export function pageReport() {
  return `
    <div class="sb-page" data-report-panel>
      <div class="wb-report-actions">
        <div class="wb-format-group" role="group" aria-label="报告格式">
          <button type="button" class="mini-btn" data-report-format="md" aria-pressed="false">Markdown</button>
          <button type="button" class="mini-btn" data-report-format="csv" aria-pressed="false">CSV</button>
        </div>
        <button type="button" class="mini-btn" data-report-copy disabled>复制报告</button>
      </div>
      <pre class="wb-report-body is-placeholder" data-report-text>选择 Markdown 或 CSV 生成用量报告。
生成后可复制内容用于记录或核对。</pre>
    </div>`;
}

/** 生成报告文本。取数走 `report_text` 命令，与 CLI 的 `agentisland report` 同一对函数。 */
export async function hydrateReportPanel(format) {
  const box = document.querySelector('[data-report-text]');
  if (!box) return;
  if (!format) return;
  const panel = box.closest('[data-report-panel]');
  panel.querySelectorAll('[data-report-format]').forEach((button) => button.setAttribute('aria-pressed', String(button.dataset.reportFormat === format)));
  panel.querySelector('[data-report-copy]').disabled = true;
  box.classList.remove('is-placeholder');
  box.textContent = '生成中…';
  try {
    const text = await invoke('report_text', { format });
    if (!box.isConnected || panel.querySelector('[aria-pressed="true"]')?.dataset.reportFormat !== format) return;
    box.textContent = text ?? '';
    panel.querySelector('[data-report-copy]').disabled = !text;
  } catch (error) {
    if (!box.isConnected || panel.querySelector('[aria-pressed="true"]')?.dataset.reportFormat !== format) return;
    box.textContent = `生成失败：${error}`;
  }
}

export function pageProvider() {
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
  const route = getState().route;
  if (route.startsWith('agentDetail:')) return pageAgentDetail(eng, route.slice('agentDetail:'.length));
  const running = eng.snapshots.filter(isVisible);
  if (running.length === 0) {
    return `<div class="wb-empty">${navigationIcon('terminal')}<strong>还没有检测到运行中的智能体</strong><span>启动本机编码工具后，运行状态会显示在这里。</span></div>`;
  }
  return running
    .map((snap) => {
      const model = agentRowModel(snap);
      const detail = [model.statusText, model.actionText || model.activityText].filter(Boolean).join(' · ');
      return `<button type="button" class="wb-agent" data-agent="${model.id}">
        <span class="wb-agent-glyph">${navigationIcon('terminal')}</span>
        <span class="name">${escapeHtml(model.name)}</span>
        <span class="tokens">${escapeHtml(model.tokensText)}</span>
        <span class="meta"><i class="wb-state-dot" style="background:${model.statusColor}"></i>${escapeHtml(detail)}</span>
      </button>`;
    })
    .join('');
}

// 工作台导航独立于智能体详情路由；周期采样只更新实时监控区。
const workbenchPages = [
  ['overview', '概览', 'square'], ['tokenAnalytics', '用量分析', 'chart'],
  ['todo', '待办事项', 'check'], ['provider', 'Codex 档位', 'sliders'],
  ['report', '导出报告', 'document'], ['agents', '智能体管理', 'terminal'],
  ['remote', '远程通知', 'bell'], ['settings', '设置', 'gear'],
];

function navigationIcon(kind) {
  const paths = {
    square: '<rect x="3" y="3" width="18" height="18" rx="5"/><path d="M9 3v18M9 10h12"/>',
    chart: '<path d="M4 20h16M6 16v-5m6 5V5m6 11V8"/>',
    check: '<rect x="3" y="3" width="18" height="18" rx="5"/><path d="m7 12 3 3 7-7"/>',
    sliders: '<path d="M4 7h6m4 0h6M4 17h10m4 0h2"/><circle cx="12" cy="7" r="2"/><circle cx="16" cy="17" r="2"/>',
    document: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9ZM14 3v6h6M8 13h8M8 17h5"/>',
    terminal: '<rect x="3" y="4" width="18" height="16" rx="4"/><path d="m7 9 3 3-3 3m6 0h4"/>',
    bell: '<path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4"/>',
    gear: '<circle cx="12" cy="12" r="3"/><path d="m9 3-1 3-3 1-2 3 2 2-1 3 3 2 3-1 2 3 3-2v-3l3-1 1-3-3-2V7l-3-1-1-3Z"/>',
  };
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[kind] ?? paths.square}</svg>`;
}

const workbenchDescriptions = {
  tokenAnalytics: '了解用量规模、趋势与模型构成。净消耗不含缓存读取。',
  todo: '把接下来的工作记在这里，完成后轻轻勾选。',
  provider: '管理本机 Codex 配置档位，查看当前配置与备份。',
  report: '将本机用量整理成便于记录和核对的报告。',
  agents: '选择需要关注的智能体，保持监控列表清晰。',
  remote: '配置通知通道，在离开电脑时接收重要状态。',
  settings: '按你的工作习惯调整外观、采样与通知。',
};

function navigationSummary(engine) {
  if (!engine) return '<span class="nav-machine-label">本机状态</span><p>等待采样</p>';
  return `<span class="nav-machine-label">本机状态</span><div class="nav-machine-values"><div><strong>${engine.snapshots.filter(isVisible).length}</strong><span>在线</span></div><div><strong>${compact(engine.grand_total.tokens24h)}</strong><span>24h tokens</span></div></div><p>数据保存在本机</p>`;
}

export function renderNavSummaryOnly() {
  const box = document.querySelector('[data-nav-summary]');
  if (box) box.innerHTML = navigationSummary(getState().engine);
}

export function renderWorkbench() {
  const st = getState();
  const eng = st.engine ?? {
    snapshots: [], grand_total: { tokens24h: 0, tokens_total: 0, cost24h: 0, cost_total: 0 },
    latest_event: null, any_working: false, has_attention: false,
  };
  const selected = st.workbenchPage ?? 'overview';
  const title = workbenchPages.find(([key]) => key === selected)?.[1] ?? '概览';
  const root = document.getElementById('root');
  const section = (heading, content, attrs = '') => `<section class="wb-section"><h2 class="wb-section-title">${heading}</h2><div class="wb-section-body" ${attrs}>${content}</div></section>`;
  let content;
  if (selected === 'overview') {
    content = `<div class="wb-intro"><div><p class="wb-eyebrow">你的本机工作空间</p><h1>工作概览</h1><p>关注正在运行的智能体，安排接下来的工作。</p></div><div class="wb-summary" data-wb-summary>${workbenchSummary(eng)}</div></div>
      <div class="wb-grid">
        <div class="wb-col wb-col-main">
          ${section('实时监控', workbenchMonitor(eng), `data-wb-monitor data-detail-route="${esc(st.route)}"`)}
          ${section('待办事项', pageTodo())}
          ${section('导出报告', pageReport())}
        </div>
        <div class="wb-col">
          ${section('用量分析', pageAnalytics(eng))}
          ${section('Codex 档位', pageProvider())}
        </div>
      </div>`;
  } else {
    const pages = {
      tokenAnalytics: () => pageAnalytics(eng), provider: pageProvider, todo: pageTodo,
      report: pageReport, settings: pageSettings, remote: pageRemote, agents: pageAgents,
    };
    const icon = workbenchPages.find(([key]) => key === selected)?.[2];
    content = `<div class="wb-single" data-workbench-page="${selected}"><div class="wb-page-heading"><span class="wb-page-icon">${navigationIcon(icon)}</span><div><h1>${title}</h1><p>${workbenchDescriptions[selected] ?? ''}</p></div></div>${pages[selected]?.() ?? ''}</div>`;
  }
  root.innerHTML = `<div class="wb">
    <nav class="wb-nav" aria-label="工作台导航">
      <div class="wb-brand">${navigationIcon('square')}<span>AgentIsland<small>本机智能体工作台</small></span></div>
      <div class="wb-nav-label">工作空间</div>
      ${workbenchPages.map(([key, label, icon], index) => `${index === 5 ? '<div class="wb-nav-label wb-nav-divider">管理</div>' : ''}<button type="button" class="wb-nav-item${selected === key ? ' is-active' : ''}" data-wb-nav="${key}" aria-current="${selected === key ? 'page' : 'false'}">${navigationIcon(icon)}<span>${label}</span></button>`).join('')}
      <div class="wb-nav-footer" data-nav-summary>${navigationSummary(st.engine)}</div>
    </nav>
    <main class="wb-main">
      <header class="wb-head" data-tauri-drag-region><span class="wb-head-title">${title}</span><div class="wb-status" data-wb-status>${workbenchStatus(st.engine)}</div><div class="wb-head-actions"><button type="button" class="mini-btn" data-wb-hide>收起窗口</button></div></header>
      <div class="wb-content${selected === 'overview' ? ' wb-overview' : ''}">${content}</div>
    </main>
  </div>`;
  if (selected === 'overview' || selected === 'tokenAnalytics') hydrateReport();
  if (selected === 'overview' || selected === 'provider') hydrateProvider();
  if (selected === 'overview' || selected === 'todo') hydrateTodo();
  if (selected === 'remote') hydrateRemote();
  if (selected === 'settings') bindSettings();
  if (selected === 'agents') bindAgents();
  bindWorkbench();
  root.querySelectorAll('[data-wb-nav]').forEach((button) => {
    button.onclick = () => {
      st.workbenchPage = button.dataset.wbNav;
      st.route = st.workbenchPage === 'tokenAnalytics' ? 'tokenAnalytics' : 'list';
      renderWorkbench();
      root.querySelector(`[data-wb-nav="${st.workbenchPage}"]`)?.focus({ preventScroll: true });
    };
  });
}

function workbenchSummary(eng) {
  const count = eng.snapshots.filter(isVisible).length;
  return `<div><strong>${count}</strong><span>在线智能体</span></div><div><strong>${compact(eng.grand_total.tokens24h)}</strong><span>24 小时 tokens</span></div>`;
}

/** 报告面板的交互：生成（两种格式）与复制。 */
function bindReportPanel() {
  const root = document.getElementById('root');
  root.querySelectorAll('[data-report-format]').forEach((el) => {
    el.onclick = () => {
      hydrateReportPanel(el.dataset.reportFormat);
    };
  });
  root.querySelector('[data-report-copy]')?.addEventListener('click', async () => {
    const box = root.querySelector('[data-report-text]');
    if (!box?.textContent) return;
    // 复制走剪贴板 API；失败要说出来，不能让按钮「点了没反应」
    try {
      await navigator.clipboard.writeText(box.textContent);
    } catch (error) {
      box.textContent = `复制失败（${error}）：内容仍在下面，手动选中即可。\n${box.textContent}`;
    }
  });
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
  const box = document.querySelector('[data-wb-monitor]');
  const eng = getState().engine;
  if (!eng) return;
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
  root.querySelector('[data-wb-hide]')?.addEventListener('click', () => {
    invoke('hide_workbench').catch(() => {});
  });
  bindReportPanel();
  bindWorkbenchAgentClicks();
}

export function renderSidebar() {
  const focusKey = document.activeElement?.dataset?.nav;
  const scrollTop = document.querySelector('.sb-body')?.scrollTop ?? 0;
  const st = getState();
  const eng = st.engine ?? {
    snapshots: [], grand_total: { tokens24h: 0, tokens_total: 0, cost24h: 0, cost_total: 0 },
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
    { key: 'agents', label: 'Agent 启停', count: 0 },
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
  } else if (running.length === 0) {
    body = '<div class="sb-empty">还没有检测到运行中的智能体</div>';
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
        const detail = [model.statusText, model.actionText || model.activityText].filter(Boolean).join(' · ');
        return `<button type="button" class="sb-agent" data-agent="${model.id}">
          <span class="name">${escapeHtml(model.name)}</span>
          <span class="tokens" style="color:${model.statusColor}">${tokens}</span>
          <span class="meta">${escapeHtml(detail)}</span>
        </button>`;
      })
      .join('');
  }

  const header = route === 'settings'
    ? { t: '高级设置', s: '改完立即生效 · 越界自动夹回' }
    : route === 'remote'
    ? { t: '远程通知', s: '密钥只进系统钥匙串' }
    : route === 'agents'
    ? { t: 'Agent 启停', s: '关掉只是不再监控，不会终止进程' }
    : route === 'tokenAnalytics'
    ? { t: 'Token 用量', s: '净消耗 · 不含缓存读取' }
    : route === 'provider'
      ? { t: 'Codex 档位', s: '切换本机已有的 provider 配置' }
      : route === 'todo'
        // 这条分支是**看着截图补的**：待办页原先落到下面的 else，表头写着「智能体 / 全部正常」。
        // 静态检查与冒烟都发现不了——它们只看「有没有报错」。
        ? { t: '待办', s: `未完成 ${st.todosPending ?? 0} 条` }
        : { t: '智能体', s: attention > 0 ? `${attention} 个等待确认` : '全部正常' };

  const root = document.getElementById('root');
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
        <div class="sb-body">${body}</div>
      </main>
    </div>`;


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
  body.innerHTML = `<div class="sb-page">${pageAgentDetail(eng, agentId)}</div>`;
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
        <span class="name">${escapeHtml(id)}</span>
        <span class="meta">已关（本拍没出现）</span>
      </label>`)
    .join('');

  return `<div class="sb-page" data-agents-root>
    <div class="sb-note">关掉某个 Agent 只是不再监控它，不会终止它的进程。变更立即生效。</div>
    <div class="sb-hint">启用后出现在监控列表中；关闭后仍可在这里重新启用。</div>
    ${rows || '<div class="sb-empty">这一拍没有采集到任何 Agent</div>'}
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
      const set = new Set(st.settings.disabled_agents);
      if (el.checked) set.delete(id); else set.add(id);
      st.settings.disabled_agents = [...set];
      await invoke('save_settings', { newSettings: st.settings }).catch(() => {});
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
  { kind: 'ntfy', label: 'ntfy 推送', fields: [
    { key: 'topicOrURL', label: '主题名', ph: 'my-agentisland-topic' } ] },
  { kind: 'customHTTP', label: '自定义 HTTP', fields: [
    { key: 'url_template', label: '地址', ph: 'https://example.com/send?key={key}' },
    { key: 'body_template', label: '正文', ph: '{title}\n{body}' },
    { key: 'useJSONBody', label: '正文用 JSON', type: 'bool' } ] },
  { kind: 'smtpEmail', label: '邮箱 SMTP（仅 465）', fields: [
    { key: 'smtp_host', label: 'SMTP 主机' },
    { key: 'smtp_port', label: '端口', type: 'number', min: 465, max: 465 },
    { key: 'smtp_user', label: '账号' },
    { key: 'smtp_to', label: '收件人' } ] },
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
  const remote = await invoke('remote_status').catch(() => null);
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
  if (remote.insecure_endpoint) notices.push(`端点不安全：${remote.insecure_endpoint}`);
  if (remote.quiet_now) notices.push('此刻落在静默时段内');
  if (remote.away_now) notices.push(`在场判定：${remote.away_reason}`);
  if (remote.unrecognized_kind) notices.push(`设置里的通道「${remote.unrecognized_kind}」认不出，已回落到 ${remote.label}`);

  root.innerHTML = `
    <div class="sb-note">${escapeHtml(remote.limitations ?? '')}</div>
    ${notices.map((n) => `<div class="sb-hint sb-warn">${escapeHtml(n)}</div>`).join('')}

    <div class="sb-group">
      <div class="sb-group-title">通道</div>
      <label class="sb-set"><span class="sb-set-label">通道</span>
        <span class="sb-set-ctl"><select data-remote-kind>
          ${REMOTE_CHANNELS.map((c) => `<option value="${c.kind}"${c.kind === remote.kind ? ' selected' : ''}>${escapeHtml(c.label)}</option>`).join('')}
        </select></span></label>
      ${channel.fields.map((f) => remoteField(channel.kind, f, cfg)).join('')}
      <label class="sb-set"><span class="sb-set-label">附带最后一条动作</span>
        <span class="sb-set-ctl"><input type="checkbox" data-remote-cfg="include_action_detail" data-kind="${channel.kind}"${cfg.include_action_detail ? ' checked' : ''}></span></label>
      <div class="sb-hint">命令内容与文件路径默认**不送出**这台机器。勾上才会一起走。</div>
    </div>

    <div class="sb-group">
      <div class="sb-group-title">密钥（存进系统钥匙串）</div>
      <div class="sb-hint">条目名 <code>${escapeHtml(remote.secret_name)}</code>；界面与日志只显示掩码，读不回真值。</div>
      <label class="sb-set"><span class="sb-set-label">密钥</span>
        <span class="sb-set-ctl"><input type="password" data-remote-secret placeholder="留空即清除" autocomplete="off"></span></label>
      <div class="sb-foot">
        <button type="button" class="mini-btn" data-remote-save-secret>保存密钥</button>
        <button type="button" class="mini-btn" data-remote-del-secret>删除</button>
        <button type="button" class="mini-btn" data-remote-preview>发送预览</button>
      </div>
      <div data-remote-out class="sb-note"></div>
    </div>

    <div class="sb-group">
      <div class="sb-group-title">发送策略</div>
      <label class="sb-set"><span class="sb-set-label">总开关</span>
        <span class="sb-set-ctl"><input type="checkbox" data-remote-policy="master_enabled"${policy.master_enabled ? ' checked' : ''}></span></label>
      <label class="sb-set"><span class="sb-set-label">任务完成</span>
        <span class="sb-set-ctl"><input type="checkbox" data-remote-policy="send_completed"${policy.send_completed ? ' checked' : ''}></span></label>
      <label class="sb-set"><span class="sb-set-label">等待确认</span>
        <span class="sb-set-ctl"><input type="checkbox" data-remote-policy="send_attention"${policy.send_attention ? ' checked' : ''}></span></label>
      <label class="sb-set"><span class="sb-set-label">消耗告警</span>
        <span class="sb-set-ctl"><input type="checkbox" data-remote-policy="send_cost_spike"${policy.send_cost_spike ? ' checked' : ''}></span></label>
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
      <div class="sb-hint">Rust 侧还没接 macOS 在场信号层，今天「人不在」一律 fail-open 判成已离开。</div>
    </div>`;
  bindRemote();
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

function bindRemote() {
  const st = getState();
  st.settings = st.settings ?? {};
  st.settings.remote_channels = st.settings.remote_channels ?? {};
  st.settings.remote_policy = st.settings.remote_policy ?? {};

  const persist = async (out) => {
    try {
      await invoke('save_settings', { newSettings: st.settings });
      out.innerHTML = '<div class="sb-hint">已保存</div>';
    } catch (error) {
      out.innerHTML = `<div class="sb-hint sb-warn">保存失败：${escapeHtml(String(error))}</div>`;
    }
  };
  const out = document.querySelector('[data-remote-out]') ?? document.createElement('div');

  document.querySelector('[data-remote-kind]')?.addEventListener('change', async (e) => {
    st.settings.remote_kind = e.target.value;
    await persist(out);
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
    const result = await invoke('remote_secret_set', { value: input?.value ?? '' })
      .catch((e) => ({ kind: 'Failed', reason: String(e) }));
    if (result.kind === 'Ok') {
      input.value = '';
      out.innerHTML = '<div class="sb-hint">密钥已写入钥匙串</div>';
      await hydrateRemote();
    } else {
      // 写入被系统拒绝时**把原因显示出来**——本应用是 ad-hoc 签名，
      // 弹窗点「始终允许」这一步用户必须自己做得到
      out.innerHTML = `<div class="sb-hint sb-warn">写入被拒：${escapeHtml(result.reason ?? '（无原因）')}</div>`;
    }
  });
  document.querySelector('[data-remote-del-secret]')?.addEventListener('click', async () => {
    await invoke('remote_secret_delete').catch(() => false);
    out.innerHTML = '<div class="sb-hint">已请求删除</div>';
    await hydrateRemote();
  });
  document.querySelector('[data-remote-preview]')?.addEventListener('click', async () => {
    const preview = await invoke('remote_preview', { args: { kind: 'attention', agentName: 'AgentIsland', seconds: 0 } })
      .catch(() => null);
    if (!preview) { out.innerHTML = '<div class="sb-hint sb-warn">预览命令没接上</div>'; return; }
    out.innerHTML = `<div class="sb-note"><pre data-preview>${escapeHtml(preview.text ?? JSON.stringify(preview, null, 2))}</pre></div>`;
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
    { key: 'shell_mode', label: '形态', type: 'select', hint: '灵动岛与侧边栏并存，默认灵动岛',
      options: [['island', '灵动岛'], ['sidebar', '侧边栏']] },
    { key: 'appearance', label: '外观', type: 'select', hint: '浅色 / 深色 / 跟随系统',
      options: [['system', '跟随系统'], ['light', '浅色'], ['dark', '深色']] },
    { key: 'dock_edge', label: '贴边位置', type: 'select',
      options: [['top', '上'], ['bottom', '下'], ['left', '左'], ['right', '右']] },
    { key: 'sidebar_edge', label: '侧边栏靠', type: 'select', hint: '宽度拖出来后会记住',
      options: [['left', '左边'], ['right', '右边']] },
    { key: 'notification_policy', label: '通知策略', type: 'select',
      options: [['standard', '标准'], ['focus', '专注免打扰'], ['silent', '完全静默']] },
  ]},
  { group: '引擎与性能', items: [
    { key: 'sample_interval', label: '有活动时采样间隔', type: 'number', unit: '秒', min: 0.5, max: 600, step: 0.5 },
    { key: 'idle_sample_interval', label: '全闲置时采样间隔', type: 'number', unit: '秒', min: 0.5, max: 600, step: 0.5,
      hint: '有活动间隔不得大于它（会自动拉平）' },
    { key: 'working_window', label: '工作判定窗口', type: 'number', unit: '秒', min: 10, max: 300, step: 5,
      hint: '该窗口内有文件写入即判为工作中' },
    { key: 'min_working_hold', label: '工作态最短保持', type: 'number', unit: '秒', min: 1, max: 300, step: 1,
      hint: '防抖：工作信号消失后至少保持这么久' },
    { key: 'active_session_window', label: '活跃会话窗口', type: 'number', unit: '秒', min: 60, max: 3600, step: 60 },
    { key: 'cpu_threshold', label: 'CPU 工作阈值', type: 'number', unit: '%', min: 1, max: 50, step: 1 },
    { key: 'collapse_delay', label: '自动收起延迟', type: 'number', unit: '秒', min: 0.2, max: 5, step: 0.1 },
    { key: 'battery_saver_enabled', label: '电池供电时降频', type: 'bool' },
    { key: 'runaway_cpu_alert', label: '持续高负载告警', type: 'bool', hint: '关掉只关告警，不影响健康度判定' },
    { key: 'runaway_cpu_threshold', label: '高负载判定阈值', type: 'number', unit: '%', min: 10, max: 100, step: 1 },
    { key: 'runaway_duration_threshold', label: '需持续多久', type: 'number', unit: '秒', min: 30, max: 3600, step: 30 },
  ]},
  { group: 'Token 与预算', items: [
    { key: 'token_alert_enabled', label: 'Token 暴涨告警', type: 'bool' },
    { key: 'token_alert_threshold', label: '暴涨阈值', type: 'number', unit: 'token/分', min: 1000, max: 10000000, step: 1000 },
    { key: 'daily_token_budget', label: '每日 Token 预算', type: 'number', unit: 'token', min: 0, max: 1000000000, step: 100000,
      hint: '0 = 未设。滚动 24 小时口径，不是自然日' },
    { key: 'budget_alert_enabled', label: '预算告警', type: 'bool', hint: '关掉只关告警，不影响用量统计' },
  ]},
  { group: '提醒', items: [
    { key: 'play_completion_sound', label: '完成提示音', type: 'bool' },
    { key: 'auto_anomalies_alert', label: '异常驻留告警', type: 'bool', hint: '关掉只关告警，不影响卡死与健康度判定' },
  ]},
  { group: '界面与系统', items: [
    { key: 'compact_view', label: '紧凑视图', type: 'bool' },
    { key: 'hide_docked_sliver', label: '收起时隐藏微细条', type: 'bool' },
    { key: 'global_hot_key_enabled', label: '全局热键', type: 'bool',
      hint: '快捷键 Cmd/Ctrl+Shift+I（展开 / 收起，与托盘同一个动作）' },
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
  const body = SETTING_FIELDS.map((group) => `
    <div class="sb-group">
      <div class="sb-group-title">${escapeHtml(group.group)}</div>
      ${group.items.map((f) => settingRow(f, s[f.key])).join('')}
    </div>`).join('');

  return `<div class="sb-page" data-settings-root>
    <div class="sb-note">改完立即生效，不需要重启。数值超出范围会被自动夹回合法区间。</div>
    ${body}
  </div>`;
}

function settingRow(field, raw) {
  const id = escapeHtml(field.key);
  const hint = field.hint ? `<div class="sb-hint">${escapeHtml(field.hint)}</div>` : '';
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
    <span class="sb-set-label">${escapeHtml(field.label)}</span>
    <span class="sb-set-ctl">${control}</span>
  </label>${hint}`;
}

/// 绑定设置页的输入。**写盘失败要明说**：静默失败会让用户以为改掉了。
export function bindSettings() {
  const st = getState();
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
          el.closest('.sb-set')?.appendChild(notice('这一栏只收数字'));
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
      st.settings[key] = value;
      try {
        await invoke('save_settings', { newSettings: st.settings });
        if (key === 'shell_mode') await invoke('set_shell_mode', { mode: String(value) });
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
            el.closest('.sb-set')?.appendChild(notice('系统没有注册开机自启，开机时不会自动运行'));
          }
        }
        // 全局热键：注册由后端每 5 秒对齐一次，这里只需把组合键写在页面上，
        // 让用户知道按哪组键——**而不是**让用户自己猜。
        if (key === 'global_hot_key_enabled' && el.checked) {
          st.hotkeyAccel = st.hotkeyAccel ?? 'Cmd/Ctrl+Shift+I';
        }
      } catch (error) {
        el.closest('.sb-set')?.appendChild(notice(`保存失败：${error}`));
      }
    };
    el.addEventListener(el.tagName === 'SELECT' ? 'change' : el.type === 'checkbox' ? 'change' : 'change', handler);
  });
}

function notice(text) {
  return `<div class="sb-hint sb-warn">${escapeHtml(text)}</div>`;
}

export function pageSettingsHeaderLabel() {
  return '高级设置';
}


/// 填档位页。三个数据源各自独立取，**任何一项失败都明说失败**，不静默留空：
/// 「读不到」与「没有档位」在界面上是两件事，混起来用户会以为自己的配置丢了。
export async function hydrateProvider() {
  const root = document.querySelector('[data-provider-root]');
  if (!root) return;
  const status = await invoke('provider_status').catch(() => null);
  if (!status) {
    root.innerHTML = '<div class="sb-empty">读不到 Codex 状态（命令没接上？）</div>';
    return;
  }
  const profiles = await invoke('provider_list_profiles').catch(() => []);
  const backups = await invoke('provider_list_backups').catch(() => []);
  renderProviderPage(root, status, profiles, backups);
  scheduleLayoutLog();
}

function renderProviderPage(root, status, profiles, backups) {
  // ① 能力边界：**逐字**用 Rust 给的那段，界面不自己编一句话
  const limitations = `<div class="sb-note" data-limitations>${escapeHtml(status.limitations ?? '')}</div>`;

  // ② 当前生效：读不到就直说读不到（不说「无」——那会被读成「没在切换」）
  const activeName = status.active_profile_id
    ? (profiles.find((profile) => profile.id === status.active_profile_id)?.name ?? status.active_profile_id)
    : null;
  // 记进状态给导航角标用（03-approach §3：Provider 角标显示当前档位名）。
  // 读不到就记空串——角标空着是对的，显示成别的名字才是撒谎。
  getState().providerActiveName = activeName ?? '';
  const activeLine = status.installed
    ? (status.active_profile_id
        ? `生效中：<b>${escapeHtml(activeName)}</b>（provider <code>${escapeHtml(status.active_provider_id ?? '')}</code>）`
        : `生效中：<b>不是本应用的档位</b>（读到的 provider 是 <code>${escapeHtml(status.active_provider_id ?? '未设置')}</code>）`)
    : '未检测到 Codex 配置（没装，或还没跑过一次）';

  if (root.closest('.wb-overview')) {
    root.innerHTML = `${limitations}<div class="sb-kv" data-status>${activeLine}</div><p class="wb-provider-count">${profiles.length} 个本机档位 · ${backups.length} 份备份</p><button type="button" class="mini-btn" data-provider-open>管理档位</button>`;
    root.querySelector('[data-provider-open]').onclick = () => {
      getState().workbenchPage = 'provider';
      getState().route = 'list';
      renderWorkbench();
    };
    return;
  }

  // ③ 档位列表：每条带「切换」，生效中的标出来
  const rows = profiles.length === 0
    ? '<div class="sb-empty">还没有档位。先在下面建一个。</div>'
    : profiles
        .map((profile) => {
          const isActive = profile.id === status.active_profile_id;
          return `<div class="sb-profile" data-profile-row="${escapeHtml(profile.id)}">
            <div class="name">${escapeHtml(profile.name)}${isActive ? '<span class="sb-tag">生效中</span>' : ''}</div>
            <div class="meta">${escapeHtml(profile.model)} · ${escapeHtml(profile.provider_id)} · ${escapeHtml(profile.base_url)}</div>
            <div class="meta">key 来自环境变量 <code>${escapeHtml(profile.env_key)}</code></div>
            <div class="actions">
              ${isActive ? '' : `<button type="button" class="mini-btn" data-switch="${escapeHtml(profile.id)}">切换到此档</button>`}
              <button type="button" class="mini-btn" data-delete="${escapeHtml(profile.id)}">删除</button>
            </div>
          </div>`;
        })
        .join('');

  // ④ 备份：还原是破坏性动作，所以也要确认
  const backupRows = backups.length === 0
    ? '<div class="sb-empty">还没有备份。第一次切换时才会产生。</div>'
    : backups
        .slice(0, 8)
        .map(
          (backup) => `<div class="sb-backup">
            <span class="name">${escapeHtml(backup.name)}</span>
            <button type="button" class="mini-btn" data-restore="${escapeHtml(backup.name)}">还原</button>
          </div>`,
        )
        .join('');

  // ⑤ 新增档位：字段与 Rust 侧的 `CodexProfile` 同名（哨兵会盯着这些名字）
  const form = `
    <div class="sb-form">
      <label class="sb-field"><span>档位标识</span><input aria-label="档位标识" data-field="id" placeholder="标识（字母数字 - _ .，例如 work）" /></label>
      <label class="sb-field"><span>档位显示名</span><input aria-label="档位显示名" data-field="name" placeholder="显示名（可留空，用标识）" /></label>
      <label class="sb-field"><span>模型</span><input aria-label="模型" data-field="model" placeholder="模型（例如 gpt-5）" /></label>
      <label class="sb-field"><span>Provider 标识</span><input aria-label="Provider 标识" data-field="provider_id" placeholder="provider 标识（例如 acme）" /></label>
      <label class="sb-field"><span>Provider 显示名</span><input aria-label="Provider 显示名" data-field="provider_name" placeholder="provider 显示名（可留空）" /></label>
      <label class="sb-field"><span>API 地址</span><input aria-label="API 地址" data-field="base_url" placeholder="base_url（https://…/v1）" /></label>
      <label class="sb-field"><span>密钥环境变量名</span><input aria-label="密钥环境变量名" data-field="env_key" placeholder="环境变量名（只存名字，不存值，例如 ACME_API_KEY）" /></label>
      <select aria-label="API 协议" data-field="wire_api"><option value="responses">responses</option><option value="chat">chat</option></select>
      <button type="button" class="mini-btn" data-save-profile>保存档位</button>
    </div>`;

  root.innerHTML = `
    ${limitations}
    <div class="sb-kv" data-status>${activeLine}</div>
    <div class="sb-section">档位</div>
    ${rows}
    <div class="sb-section">新增档位</div>
    ${form}
    <div class="sb-section">备份（切换前自动生成）</div>
    ${backupRows}
    <div class="sb-confirm" data-confirm hidden></div>`;

  bindProviderEvents(root, status, profiles);
}

function bindProviderEvents(root, status, profiles) {
  const confirmBox = root.querySelector('[data-confirm]');

  const askConfirm = (text, onConfirm) => {
    confirmBox.hidden = false;
    confirmBox.innerHTML = `<div class="text">${text}</div>
      <div class="actions"><button type="button" class="mini-btn" data-yes>确认</button><button type="button" class="mini-btn" data-no>取消</button></div>`;
    confirmBox.querySelector('[data-yes]').onclick = async () => {
      confirmBox.hidden = true;
      await onConfirm();
    };
    confirmBox.querySelector('[data-no]').onclick = () => {
      confirmBox.hidden = true;
    };
  };

  root.querySelectorAll('[data-switch]').forEach((el) => {
    el.onclick = () => {
      const id = el.dataset.switch;
      const profile = profiles.find((p) => p.id === id);
      // 确认框里写清**会发生什么**：改哪个文件、模型与 provider 会变成什么
      askConfirm(
        `把 <code>${escapeHtml(profile?.model ?? '')}</code> / <code>${escapeHtml(profile?.provider_id ?? '')}</code> ` +
          `写进 <code>${escapeHtml(status.config_path ?? '')}</code>？<br/>` +
          '切换前会先备份；正在运行的 Codex 需要重启才会用上新配置。',
        async () => {
          // 变量名 `applied` 是**约定的**：`models.rs` 的档位字段哨兵按 `applied.` / `status.` /
          // `profile.` / `backup.` 四个前缀扫这个文件，并断言这些键真的在 DTO 里。
          // 换个名字就等于把这段代码移出哨兵的保护面。
          //
          // 失败也**不挂在 `applied.` 上**：那会把一个客户端临时对象混进「DTO 字段」的地盘，
          // 哨兵会（正确地）报「DTO 里没有这个键」。失败单独一个变量。
          let failure = '';
          const applied = await invoke('provider_apply_profile', { id }).catch((error) => {
            failure = String(error);
            return null;
          });
          if (failure || !applied) {
            await hydrateProvider();
            showProviderToast(root, `切换失败：${failure || '命令没有返回结果'}`);
            return;
          }
          await hydrateProvider();
          showProviderToast(
            root,
            `已切换，备份：${applied.backup_name}。${applied.limitations ?? ''}`,
          );
        },
      );
    };
  });

  root.querySelectorAll('[data-restore]').forEach((el) => {
    el.onclick = () => {
      const name = el.dataset.restore;
      askConfirm(
        `用备份 <code>${escapeHtml(name)}</code> 覆盖当前 <code>config.toml</code>？<br/>这会丢掉备份之后的改动。`,
        async () => {
          const error = await invoke('provider_restore_backup', { name }).catch((e) => String(e));
          await hydrateProvider();
          showProviderToast(root, error ? `还原失败：${error}` : `已从 ${name} 还原`);
        },
      );
    };
  });

  root.querySelectorAll('[data-delete]').forEach((el) => {
    el.onclick = () => {
      const id = el.dataset.delete;
      askConfirm(`删除档位 <code>${escapeHtml(id)}</code>？（不会动 config.toml）`, async () => {
        const error = await invoke('provider_delete_profile', { id }).catch((e) => String(e));
        await hydrateProvider();
        showProviderToast(root, error ? `删除失败：${error}` : `已删除 ${id}`);
      });
    };
  });

  const saveBtn = root.querySelector('[data-save-profile]');
  if (saveBtn) {
    saveBtn.onclick = async () => {
      const profile = {};
      root.querySelectorAll('[data-field]').forEach((el) => {
        profile[el.dataset.field] = el.value.trim();
      });
      const saved = await invoke('provider_save_profile', { profile }).catch((e) => ({ error: String(e) }));
      await hydrateProvider();
      // 校验在 Rust 侧：**原话带回界面**，不在这里翻译成自己的说法
      showProviderToast(root, saved?.error ? `保存失败：${saved.error}` : `已保存档位 ${saved.id}`);
    };
  }
}

function showProviderToast(root, text) {
  const toast = root.querySelector('[data-toast]') ?? document.createElement('div');
  toast.className = 'sb-toast';
  toast.setAttribute('data-toast', '');
  toast.textContent = text;
  root.appendChild(toast);
  // 同上：Toast 会随重画消失，失败信息另写一份到应用日志
  invoke('log_from_ui', { message: `[provider] ${text}` }).catch(() => {});
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
  const todo = await invoke('todos_list').catch(() => null);
  if (!todo) {
    root.innerHTML = '<div class="sb-empty">读不到待办清单</div>';
    return;
  }
  renderTodoPage(root, todo);
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
      <input aria-label="新增待办" data-todo-input placeholder="加一条待办，回车确认" maxlength="500" />
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
