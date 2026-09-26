// 灵动岛视图渲染（IslandView / AgentRowView / TokenSummaryBar / SubViews 的 Web 对应物）
import { invoke } from './tauri.js';
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
export function costText(c) { return c > 0.005 ? `$${c.toFixed(2)}` : ''; }

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
    color = snap.isHung ? 'var(--ring-red)' : (snap.cpu_percent ?? 0) >= 80 ? 'var(--ring-orange)' : 'var(--ring-green)';
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
    return {
      title: ev.agent_name,
      subtitle: eventSummary(ev),
      badge: ev.externally_delivered ? (isCost ? '外部告警' : '外部确认') : isCost ? '告警' : '待确认',
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

function rowHtml(snap) {
  const c = levelColors(snap.level);
  const uncertainLabels = {
    blindSessionSource: '会话源读不到',
    noLocalData: '无本地明细',
    sourceNotWired: '未接入明细源',
  };
  const uncertainty = snap.level === 'idle' ? uncertainLabels[snap.observability?.code] : null;
  const pillColor = uncertainty ? 'var(--warning)' : c.fg;
  const pillBackground = uncertainty ? 'color-mix(in srgb, var(--warning) 10%, transparent)' : c.bg;
  const pillBorder = uncertainty ? 'color-mix(in srgb, var(--warning) 25%, transparent)' : c.border;
  const hasAction = ['working', 'attention'].includes(snap.level) && snap.current_action;
  const usage = snap.token_usage && snap.token_usage.tokens24h > 0;
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
        ${`<span class="status-pill" title="${esc(snap.observability?.summary ?? snap.level_label)}" style="color:${pillColor};background:${pillBackground};border-color:${pillBorder}">${esc(uncertainty ?? snap.level_label)}</span>`}
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
        ${costText(gt.cost24h) ? `<span class="cost">${costText(gt.cost24h)}</span>` : ''}
      </div>
      <div class="right">
        <span class="mini-tag" style="color:var(--text-muted);background:rgba(127,127,127,0.10);border-color:var(--hairline)">TOTAL</span>
        <span class="num">${compact(gt.tokens_total)}</span>
        ${costText(gt.cost_total) ? `<span class="cost">${costText(gt.cost_total)}</span>` : ''}
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
        <div><div class="big-num c-working" style="font-size:14px">$${(u.cost24h * remaining).toFixed(2)}</div><div class="num-label">预估月末费用</div></div>
        <div><div class="big-num" style="font-size:14px">${remaining} 天</div><div class="num-label">当月剩余自然日</div></div>
      </div>
    </div>
    <div class="card-box">
      <div class="totals">
        <div><div class="big-num">${compact(u.tokens24h)}</div><div class="num-label">24h 用量</div></div>
        <div><div class="big-num">${costText(u.cost24h) || '—'}</div><div class="num-label">费用</div></div>
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
            <span class="r"><span class="tk">${compact(m.tokens)}</span><span class="cost">${costText(m.cost)}</span></span></div>
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
        <div><div class="big-num" style="font-size:14px">${costText(u.cost24h) || '—'}</div><div class="num-label">24h 花费</div></div>
        <div><div class="big-num" style="font-size:14px">${compact(u.tokens_total)}</div><div class="num-label">累计</div></div>
        <div><div class="big-num" style="font-size:14px">${costText(u.cost_total) || '—'}</div><div class="num-label">累计花费</div></div>
      </div>
    </div>
    <div class="card-box">
      <h4>按模型拆分（24h）</h4>
      ${report.models24h.map((m) => {
    const maxT = Math.max(1, ...report.models24h.map((x) => x.tokens));
    return `<div class="model-row" data-model="${esc(m.model)}">
          <div class="top"><span>${esc(m.model)}</span>
            <span class="r"><span class="tk">${compact(m.tokens)}</span><span class="cost">${costText(m.cost)}</span></span></div>
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
    loadReport();
  }));

  const jump = root.querySelector('[data-agent-jump]');
  if (jump) jump.addEventListener('click', (e) => {
    e.stopPropagation();
    // 直达窗口：Windows 端后续接 Win32 激活；v1 回到详情页
    st.route = `agentDetail:${jump.dataset.agentJump}`;
    renderCard();
    loadReport();
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
  if (st.route === 'tokenAnalytics' || st.route.startsWith('agentDetail:')) loadReport();
}

function bindRowClicks(st) {
  document.querySelectorAll('[data-agent]').forEach((el) => {
    el.onclick = () => {
      st.route = `agentDetail:${el.dataset.agent}`;
      renderCard();
      loadReport();
    };
  });
}

async function loadReport() {
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
  // 报告注入后内容高度变化，窗口跟随
  await resizeToContent();
}
