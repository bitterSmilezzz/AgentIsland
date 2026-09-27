// 灵动岛视图渲染（IslandView / AgentRowView / TokenSummaryBar / SubViews 的 Web 对应物）
import { invoke } from './tauri.js';
import { isIsland } from './shell.js';
import { getState, setState, expand, collapse, armCollapseTimer, scheduleRender, resizeToContent, applyAppearance, applyEdge } from './main.js';

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

export function sliverSize(edge) {
  return edge === 'left' || edge === 'right'
    ? { w: 18, h: 132 }
    : { w: 152, h: 20 };
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

  root.innerHTML = `
    <div class="sliver ${vertical ? 'vertical' : ''} ${hitClass} ${working ? 'working' : ''} ${alert ? 'alert' : ''}" id="sliver">
      <div class="sliver-capsule ${working || alert ? 'breathing' : ''}"></div>
    </div>`;

  const el = root.querySelector('#sliver');
  el.addEventListener('mouseenter', () => expand());
  el.addEventListener('mouseup', () => expand());
}

// MARK: 顶栏状态摘要（HeaderPresentation）

function headerPresentation(eng) {
  const visible = eng?.snapshots.filter((s) => s.process_running) ?? [];
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
  const c = levelColors(snap.level);
  const pillColor = model.statusColor;
  const pillBackground = model.statusBackground;
  const pillBorder = model.statusBorder;
  const hasAction = model.hasAction;
  const usage = model.tokensText !== '—';
  const dots = [10, 60, 300, 900, 3600];
  const ago = snap.last_activity_text === '刚刚' ? 5 : null;

  const actionColor = snap.level === 'attention' ? 'var(--warning)' : 'var(--working)';
  const barStyle = `color:${actionColor};background:color-mix(in srgb, ${actionColor} 8%, transparent);border:0.5px solid color-mix(in srgb, ${actionColor} 22%, transparent)`;

  return `
  <div class="row" data-agent="${esc(snap.id)}">
    <div class="row-line1">
      <div class="row-left">
        ${ringHtml(snap, 36)}
        <div class="row-name-col">
          <div class="row-name">${esc(snap.name)}</div>
          <div class="row-sub">
            ${usage
              ? `<span class="token-badge"><span class="bolt">⚡</span>${compact(snap.token_usage.tokens24h)}</span>`
              : `<span class="last-activity">${esc(snap.last_activity_text)}</span>`}
            <span class="activity-dots">${dots.map((t, i) =>
              `<i class="${snap.level === 'working' && i < 2 ? 'on' : (ago !== null && ago < t ? 'on' : '')}"></i>`).join('')}</span>
          </div>
        </div>
      </div>
      <div class="row-right">
        ${snap.process_running && snap.memory_bytes > 0 ? `<span class="mem-badge" title="物理内存驻留集 (RSS): ${esc(snap.memory_text)}">${esc(snap.memory_text)}</span>` : ''}
        ${healthChip(snap)}
        ${`<span class="status-pill" title="${esc(snap.observability?.summary ?? model.statusText)}" style="color:${pillColor};background:${pillBackground};border-color:${pillBorder}">${esc(model.statusText)}</span>`}
      </div>
    </div>
    ${hasAction ? `
    <div class="action-bar" style="${barStyle}" title="${esc(snap.current_action)}">
      <span class="ic">${snap.level === 'attention' ? '\uE7C2' : '\uE756'}</span>
      <span class="txt">${esc(snap.current_action)}</span>
      ${snap.subagent_count > 0 ? `<span style="color:var(--cyan);font-family:var(--font-text);font-size:8px;font-weight:700;background:color-mix(in srgb, var(--cyan) 18%, transparent);padding:1px 4px;border-radius:999px">${snap.subagent_count}子任务</span>` : ''}
    </div>` : ''}
  </div>`;
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
  const visible = eng.snapshots.filter((s) => s.process_running);
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
  const shelfSnaps = visible.filter((s) => ['attention', 'working'].includes(s.level)).slice(0, 6);
  const gt = eng.grand_total ?? {};
  const hasSummary = (gt.tokens24h ?? 0) > 0 || (gt.tokens_total ?? 0) > 0;
  const ev = eng.latest_event;

  const chev = edge === 'top' ? ICONS.chevUp : edge === 'bottom' ? ICONS.chevDown : edge === 'left' ? ICONS.chevLeft : ICONS.chevRight;
  const themeIcon = dark ? ICONS.sun : ICONS.moon;

  const shelf = shelfSnaps.length > 0 ? `
    <div class="divider"></div>
    <div class="shelf">${shelfSnaps.map((s) => {
      const sub = s.level === 'working' ? '工作中' : s.level === 'attention' ? '等待你确认' : s.level === 'completed' ? '任务已完成'
        : s.token_usage?.tokens24h > 0 ? compact(s.token_usage.tokens24h) : s.level_label;
      const sc = s.level === 'attention' ? 'var(--warning)' : ['working', 'completed'].includes(s.level) ? 'var(--ring-green)' : 'var(--text-faint)';
      return `<div class="shelf-chip" data-agent="${esc(s.id)}">
        ${ringHtml(s, 30)}
        <div><div class="nm">${esc(s.name)}</div><div class="sub" style="color:${sc}">${esc(sub)}</div></div>
      </div>`;
    }).join('')}</div>` : '';

  const banner = ev ? `
    <div class="divider"></div>
    <div class="banner" style="background:color-mix(in srgb, ${ev.event_type === 'costSpike' ? 'var(--danger)' : 'var(--warning)'} 10%, transparent)">
      <div class="line1">
        <span class="icon" style="color:${ev.event_type === 'costSpike' ? 'var(--danger)' : 'var(--warning)'}">${ev.event_type === 'completed' ? '\uE73E' : '\uE7BA'}</span>
        <span class="title" style="color:${ev.event_type === 'costSpike' ? 'var(--danger)' : 'var(--warning)'}">${esc(bannerTitle(ev))}</span>
        <span class="mini-btn" data-banner-detail>${st.lastEventDetail ? '原因 ˄' : '原因 ˅'}</span>
        <span class="mini-btn" data-banner-close style="padding:2px 5px">${ICONS.close}</span>
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
    ? `<div class="empty">${st.searchActive ? `<span style="font-size:18px">🔍</span><span>未找到匹配「${esc(st.searchText)}」的智能体</span>`
      : `<span style="font-size:22px">💤</span><span>没有活跃的 Agent</span>`}</div>`
    : `<div class="list">${filtered.map(rowHtml).join('')}</div>`;

  const summary = hasSummary ? `
    <div class="divider"></div>
    <div class="summary" data-analytics>
      <div class="left">
        <span class="mini-tag" style="color:var(--cyan);background:color-mix(in srgb, var(--cyan) 16%, transparent);border-color:color-mix(in srgb, var(--cyan) 35%, transparent)">24H</span>
        <span class="num">${compact(gt.tokens24h)}</span>
        ${costText(gt.cost24h, gt.cost_estimated) ? `<span class="cost">${costText(gt.cost24h, gt.cost_estimated)}</span>` : ''}
      </div>
      <div class="right">
        <span class="mini-tag" style="color:var(--text-muted);background:rgba(127,127,127,0.10);border-color:var(--hairline)">TOTAL</span>
        <span class="num">${compact(gt.tokens_total)}</span>
        ${costText(gt.cost_total, gt.cost_estimated) ? `<span class="cost">${costText(gt.cost_total, gt.cost_estimated)}</span>` : ''}
        <span class="chev">›</span>
      </div>
    </div>` : '';

  const statusColor = eng.has_attention ? 'var(--warning)' : eng.any_working ? 'var(--working)' : 'var(--idle)';

  return `
    <div class="header" data-drag>
      <div class="status-dot" style="background:${statusColor}">${eng.any_working ? '<span class="pulse" style="background:' + statusColor + '"></span>' : ''}</div>
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
        <span class="header-count">${visible.length}</span>
        <span class="icon-btn" data-search title="即时搜索过滤 (/)">${ICONS.search}</span>
        <span class="icon-btn" data-theme title="外观主题">${themeIcon}</span>
        <span class="icon-btn" data-analytics title="Token 用量分析">${ICONS.wrench}</span>
        <span class="icon-btn" data-collapse title="收起灵动岛">${chev}</span>
      </div>
    </div>
    <div class="divider"></div>
    ${shelf}
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
        <span class="back-btn" data-back>‹</span>
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
    <div class="card-box">
      <h4>📈 月末用量与成本预测</h4>
      <div class="totals" style="margin-top:8px;justify-content:flex-start;gap:14px">
        <div><div class="big-num" style="font-size:14px">${compact(u.tokens24h * remaining)}</div><div class="num-label">预估月末消耗</div></div>
        <div><div class="big-num c-working" style="font-size:14px">${costText(u.cost24h, u.cost_estimated) ? `~$${(u.cost24h * remaining).toFixed(2)}` : '—'}</div><div class="num-label">预估月末费用</div></div>
        <div><div class="big-num" style="font-size:14px">${remaining} 天</div><div class="num-label">当月剩余自然日</div></div>
      </div>
    </div>
    <div class="card-box">
      <div class="totals">
        <div><div class="big-num">${compact(u.tokens24h)}</div><div class="num-label">24h 用量</div></div>
        <div><div class="big-num">${costText(u.cost24h, u.cost_estimated) || '—'}</div><div class="num-label">费用</div></div>
        <div><div class="big-num">${compact(u.tokens_total)}</div><div class="num-label">累计</div></div>
      </div>
    </div>
    <div class="card-box">
      <div style="display:flex;align-items:center"><h4>使用趋势</h4>
        <span style="margin-left:auto;font-size:9.5px;color:var(--cyan);font-family:var(--font-mono)">峰值 ${compact(max)}</span></div>
      <svg width="${W}" height="${H}" style="margin-top:6px">
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
    <div class="card-box">
      <div style="display:flex;align-items:center"><span style="font-size:10.5px;color:var(--text-faint)">24h 协同节律</span>
        <span style="margin-left:auto;font-size:9.5px;color:var(--text)">活跃 ${heat.filter((h) => !h.includes('8%')).length}/24h</span></div>
      <div class="heat">${heat.join('')}</div>
    </div>
    <div class="card-box">
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
    <div class="page" data-page="agentDetail">
      <div class="page-header">
        <span class="back-btn" data-back>‹</span>
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
        const visible = (st.engine?.snapshots ?? []).filter((s) => s.process_running);
        const filtered = visible.filter((s) => s.name.toLowerCase().includes(st.searchText.toLowerCase()) || s.id.includes(st.searchText.toLowerCase()));
        listZone.innerHTML = filtered.length === 0
          ? `<div class="empty" style="padding:24px 0"><span style="font-size:18px">🔍</span><span>未找到匹配「${esc(st.searchText)}」的智能体</span></div>`
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
  const body = document.querySelector('[data-report-root]');
  if (!body) return;
  const agentId = st.route.startsWith('agentDetail:') ? st.route.split(':')[1] : st.engine?.snapshots?.[0]?.id ?? '';
  const report = await invoke('get_report', { agentId }).catch(() => null);
  if (!report) {
    body.innerHTML = '<div class="c-faint" style="font-size:11px;text-align:center;padding:20px 0">暂无本地明细数据</div>';
    return;
  }
  const snap = st.engine?.snapshots.find((s) => s.id === agentId);
  body.innerHTML = st.route === 'tokenAnalytics' ? renderReportBody(report) : renderDetailBody(report, snap);
  // 报告注入后内容高度变化，窗口跟随——**只在灵动岛窗口做**：
  // 侧边栏是一整列固定尺寸的窗口，跟着内容长高会把用户拉好的宽度与位置一起改掉
  if (isIsland()) await resizeToContent();
}

// MARK: - 侧边栏形态

/// 侧边栏壳：左导航 + 右内容。
///
/// 与灵动岛的**数据与页面函数完全共用**，只是容器不同——这就是「监控模块不依赖容器尺寸」
/// 这句要求的最小落地：`pageAnalytics` / `pageAgentDetail` 原样复用，
/// 侧边栏自己只负责导航与列表。
export function renderSidebar() {
  const st = getState();
  const eng = st.engine ?? {
    snapshots: [], grand_total: { tokens24h: 0, tokens_total: 0, cost24h: 0, cost_total: 0 },
    latest_event: null, any_working: false, has_attention: false,
  };
  const running = eng.snapshots.filter((snap) => snap.process_running || snap.level !== 'offline');
  const attention = eng.snapshots.filter((snap) => snap.level === 'attention').length;

  const nav = [
    { key: 'list', label: '监控', count: running.length },
    { key: 'tokenAnalytics', label: 'Token 用量', count: 0 },
    { key: 'provider', label: 'Codex 档位', count: 0 },
    // 角标是**未完成**数（不是总条数）：勾掉最后一条之后角标就该消失
    { key: 'todo', label: '待办', count: st.todosPending ?? 0, badge: 'todo' },
    // 03-approach §3：首层极简，重页面（设置 / 分析 / 详情）从这里进
    { key: 'settings', label: '高级设置 ›', count: 0 },
  ];
  const route = ['list', 'tokenAnalytics', 'provider', 'todo', 'settings'].includes(st.route) ? st.route : 'list';

  let body;
  if (route === 'settings') {
    body = pageSettings();
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
        return `<div class="sb-agent" data-agent="${model.id}">
          <div class="name">${escapeHtml(model.name)}</div>
          <div class="tokens" style="color:${model.statusColor}">${tokens}</div>
          <div class="meta">${escapeHtml(detail)}</div>
        </div>`;
      })
      .join('');
  }

  const header = route === 'settings'
    ? { t: '高级设置', s: '改完立即生效 · 越界自动夹回' }
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
      <nav class="sb-nav">
        <div class="sb-brand">AgentIsland</div>
        ${nav
          .map(
            (item) => `<div class="sb-item${item.key === route ? ' is-active' : ''}${item.disabled ? ' is-disabled' : ''}"
              ${item.disabled ? '' : `data-nav="${item.key}"`}>
              <span>${escapeHtml(item.label)}</span>${item.text
                ? `<span class="sb-count" data-nav-text="${escapeHtml(item.badge ?? '')}">${escapeHtml(item.text)}</span>`
                : item.count ? `<span class="sb-count" ${item.badge ? `data-nav-count="${item.badge}"` : ''}>${item.count}</span>` : ''}
            </div>`,
          )
          .join('')}
      </nav>
      <main class="sb-main">
        <div class="sb-head"><div class="t">${header.t}</div><div class="s">${header.s}</div></div>
        <div class="sb-body">${body}</div>
      </main>
    </div>`;


  if (route === 'settings') bindSettings();

  // 量一次布局（在内容就位之后：把度量放在注水之前只会量到「加载中…」的骨架，
  // 我第一版就是这么量的，于是每个路由的数字都一模一样、看起来「都没问题」）。
  scheduleLayoutLog();

  root.querySelectorAll('[data-nav]').forEach((el) => {
    el.onclick = async () => {
      st.route = el.dataset.nav;
      renderSidebar();
      if (st.route === 'tokenAnalytics') await hydrateReport();
      if (st.route === 'provider') await hydrateProvider();
      if (st.route === 'todo') await hydrateTodo();
      if (st.route === 'settings') bindSettings();
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
    { key: 'global_hot_key_enabled', label: '全局热键', type: 'bool' },
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
        if (key === 'appearance') applyAppearance(String(value));
        if (key === 'dock_edge') applyEdge();
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


  return `
    <div class="sb-page" data-provider-root>
      <div class="sb-empty">加载中…</div>
    </div>`;
}

/// 填档位页。三个数据源各自独立取，**任何一项失败都明说失败**，不静默留空：
/// 「读不到」与「没有档位」在界面上是两件事，混起来用户会以为自己的配置丢了。
export async function hydrateProvider() {  const root = document.querySelector('[data-provider-root]');
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
              ${isActive ? '' : `<span class="mini-btn" data-switch="${escapeHtml(profile.id)}">切换到此档</span>`}
              <span class="mini-btn" data-delete="${escapeHtml(profile.id)}">删除</span>
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
            <span class="mini-btn" data-restore="${escapeHtml(backup.name)}">还原</span>
          </div>`,
        )
        .join('');

  // ⑤ 新增档位：字段与 Rust 侧的 `CodexProfile` 同名（哨兵会盯着这些名字）
  const form = `
    <div class="sb-form">
      <input data-field="id" placeholder="标识（字母数字 - _ .，例如 work）" />
      <input data-field="name" placeholder="显示名（可留空，用标识）" />
      <input data-field="model" placeholder="模型（例如 gpt-5）" />
      <input data-field="provider_id" placeholder="provider 标识（例如 acme）" />
      <input data-field="provider_name" placeholder="provider 显示名（可留空）" />
      <input data-field="base_url" placeholder="base_url（https://…/v1）" />
      <input data-field="env_key" placeholder="环境变量名（只存名字，不存值，例如 ACME_API_KEY）" />
      <select data-field="wire_api"><option value="responses">responses</option><option value="chat">chat</option></select>
      <span class="mini-btn" data-save-profile>保存档位</span>
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
      <div class="actions"><span class="mini-btn" data-yes>确认</span><span class="mini-btn" data-no>取消</span></div>`;
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
  // 读坏过就**说出来**：清单看起来是空的，但用户的东西并没有被删掉（留档了）
  const broken = todo.broken_backup
    ? `<div class="sb-note">上次的清单读不出来，已留档为 <code>${escapeHtml(todo.broken_backup)}</code>。当前显示的是空清单。</div>`
    : '';

  const rows = todo.items.length === 0
    ? '<div class="sb-empty">还没有待办。在下面输入，回车即可加一条。</div>'
    : todo.items
        .map(
          (item) => `<div class="sb-todo${item.done ? ' is-done' : ''}" data-todo-row="${escapeHtml(item.id)}">
            <span class="box" data-toggle="${escapeHtml(item.id)}">${item.done ? '✓' : ''}</span>
            <span class="text">${escapeHtml(item.text)}</span>
            <span class="x" data-remove="${escapeHtml(item.id)}">×</span>
          </div>`,
        )
        .join('');

  const clearBtn = todo.items.some((item) => item.done)
    ? '<span class="mini-btn" data-clear-done>清除已完成</span>'
    : '';

  root.innerHTML = `
    ${broken}
    ${rows}
    <div class="sb-todo-add">
      <input data-todo-input placeholder="加一条待办，回车确认" maxlength="500" />
    </div>
    <div class="sb-todo-foot">${clearBtn}</div>`;

  updateTodoBadge(root, todo.pending);

  // 回车加条：极简列表的全部交互就是这一下
  const input = root.querySelector('[data-todo-input]');
  if (input) {
    input.onkeydown = async (event) => {
      if (event.key !== 'Enter') return;
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
    input.focus();
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
