// Local, static identity marks: no icon font, runtime CDN or status-dependent color.
// SVG provenance and license: ../assets/agents/README.md.
export const agentIdentities = Object.freeze({
  dim: ['DimAgent', 'dim', '#7b5a94', '#c0a6d6', 'dim.png'],
  zcode: ['ZCode', 'zcode', '#496699', '#a3bce7', 'zcode.png'],
  claude: ['Claude', 'claude', '#a55d40', '#e3a58a'],
  codex: ['ChatGPT / Codex', 'codex', undefined, undefined, 'chatgpt.png'],
  cursor: ['Cursor', 'cursor'],
  vscode: ['VS Code', 'vscode', '#326c99', '#8fbce0'],
  cline: ['Cline', 'cline'],
  'roo-code': ['Roo Code', 'roocode'],
  opencode: ['OpenCode', 'opencode'],
  minimaxcode: ['MiniMax Code', 'minimax', '#ad4e7d', '#e8a3c8', 'minimaxcode.png'],
  mimocode: ['Xiaomi MiMo', 'xiaomimimo', '#9c6432', '#ddb28b', 'mimodesktop.png'],
  goose: ['Goose', 'goose', '#716642', '#cbc09b'],
  aider: ['Aider', 'aider', '#476d58', '#9ac5ac'],
  windsurf: ['Windsurf', 'windsurf', '#38786f', '#96c8be'],
  trae: ['Trae', 'trae', '#3f7559', '#98c7aa', 'trae.png'],
  qoder: ['Qoder', 'qoder', '#715b9b', '#bba8db', 'qoder.png'],
  // This registry id is Tencent ima.copilot, not GitHub Copilot.
  copilot: ['ima.copilot', 'ima', '#5b6898', '#abb7df', 'ima.svg'],
  workbuddy: ['WorkBuddy', 'workbuddy', '#4c7490', '#a0c1d8', 'workbuddy.png'],
  'workbuddy-ai': ['WorkBuddy AI', 'workbuddyai', '#6a6091', '#bcb0d7', 'workbuddyai.png'],
  antigravity: ['Antigravity', 'antigravity', '#566a9c', '#a4b8e0'],
  hermes: ['Hermes Agent', 'hermesagent', '#92733f', '#d6c091'],
  continue: ['Continue', 'continue', '#52715b', '#a9c6af'],
  dsh: ['DeepSeek Harness', 'deepseek', '#4a65a2', '#a6bbed', 'dsh.svg'],
  'vibe-usage': ['Vibe Usage', 'vibeusage', '#806946', '#d1bb96', 'vibeusage.png'],
  openviking: ['OpenViking', 'openviking', '#616993', '#b1b9de', 'openviking.svg'],
});

const esc = value => String(value ?? '').replace(/[&<>"']/g, ch =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[ch]));

export function agentIcon(agent, extraClass = '') {
  const identity = Object.prototype.hasOwnProperty.call(agentIdentities, agent?.id)
    ? agentIdentities[agent.id] : undefined;
  const mark = identity?.[1] || 'customagent';
  const file = identity?.[4];
  const ink = identity?.[2] ? ` style="--agent-light:${identity[2]};--agent-dark:${identity[3]}"` : '';
  const content = file
    ? `<img class="agent-brand-image${['ima.svg', 'dsh.svg'].includes(file) ? ' agent-brand-image--backed' : ''}" src="${esc(new URL(`../assets/agents/${file}`, import.meta.url).href)}" alt="" draggable="false">`
    : `<span class="agent-mark" style="--agent-mask:url('${esc(new URL(`../assets/agents/${mark}.svg`, import.meta.url).href)}')"></span>`;
  return `<span class="agent-avatar${file ? ' agent-avatar--brand' : ''} ${esc(extraClass)}" data-agent-icon="${esc(agent?.id || '')}" aria-hidden="true"${ink}>${content}</span>`;
}
