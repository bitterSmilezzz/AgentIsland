// Local, static identity marks: no icon font, runtime CDN or status-dependent color.
// SVG provenance and license: ../assets/agents/README.md.
export const agentIdentities = Object.freeze({
  dim: ['DimAgent', 'DI', '#7b5a94', '#c0a6d6'],
  zcode: ['ZCode', 'ZC', '#496699', '#a3bce7'],
  claude: ['Claude', 'claude', '#a55d40', '#e3a58a'],
  codex: ['ChatGPT / Codex', 'codex'],
  cursor: ['Cursor', 'cursor'],
  vscode: ['VS Code', 'VS', '#326c99', '#8fbce0'],
  cline: ['Cline', 'cline'],
  'roo-code': ['Roo Code', 'roocode'],
  opencode: ['OpenCode', 'opencode'],
  mimocode: ['Xiaomi MiMo', 'xiaomimimo', '#9c6432', '#ddb28b'],
  goose: ['Goose', 'goose', '#716642', '#cbc09b'],
  aider: ['Aider', 'AD', '#476d58', '#9ac5ac'],
  windsurf: ['Windsurf', 'windsurf', '#38786f', '#96c8be'],
  trae: ['Trae', 'trae', '#3f7559', '#98c7aa'],
  qoder: ['Qoder', 'qoder', '#715b9b', '#bba8db'],
  // This registry id is Tencent ima.copilot, not GitHub Copilot.
  copilot: ['ima.copilot', 'IM', '#5b6898', '#abb7df'],
  workbuddy: ['WorkBuddy', 'WB', '#4c7490', '#a0c1d8'],
  'workbuddy-ai': ['WorkBuddy AI', 'WA', '#6a6091', '#bcb0d7'],
  antigravity: ['Antigravity', 'antigravity', '#566a9c', '#a4b8e0'],
  hermes: ['Hermes Agent', 'hermesagent', '#92733f', '#d6c091'],
  continue: ['Continue', 'CT', '#52715b', '#a9c6af'],
  dsh: ['DeepSeek Harness', 'deepseek', '#4a65a2', '#a6bbed'],
  'vibe-usage': ['Vibe Usage', 'VU', '#806946', '#d1bb96'],
  openviking: ['OpenViking', 'OV', '#616993', '#b1b9de'],
});

const esc = value => String(value ?? '').replace(/[&<>"']/g, ch =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[ch]));

export function agentIcon(agent, extraClass = '') {
  const identity = Object.prototype.hasOwnProperty.call(agentIdentities, agent?.id)
    ? agentIdentities[agent.id] : undefined;
  const words = String(agent?.name || agent?.id || 'AI').trim().split(/\s+/u);
  const initials = words.length > 1
    ? words.slice(0, 2).map(word => Array.from(word)[0]).join('')
    : Array.from(words[0]).slice(0, 2).join('');
  const mark = identity?.[1];
  // Catalog marks with lowercase names are the vendored SVGs; all other entries are initials.
  const svg = mark && /^[a-z]+$/.test(mark);
  const ink = identity?.[2] ? ` style="--agent-light:${identity[2]};--agent-dark:${identity[3]}"` : '';
  const content = svg
    ? `<span class="agent-mark" style="--agent-mask:url('${esc(new URL(`../assets/agents/${mark}.svg`, import.meta.url).href)}')"></span>`
    : `<span class="agent-monogram">${esc(mark || initials.toLocaleUpperCase())}</span>`;
  return `<span class="agent-avatar ${esc(extraClass)}" data-agent-icon="${esc(agent?.id || '')}" aria-hidden="true"${ink}>${content}</span>`;
}
