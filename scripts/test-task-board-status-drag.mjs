import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, extname, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const { chromium } = await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const mainRs = await readFile(new URL('../app/src-tauri/src/main.rs', import.meta.url), 'utf8');
const tasksRs = await readFile(new URL('../app/src-tauri/src/tasks.rs', import.meta.url), 'utf8');
const handlerStart = mainRs.indexOf('tauri::generate_handler![');
const handlerEnd = mainRs.indexOf('])', handlerStart);
assert.ok(handlerStart >= 0 && handlerEnd > handlerStart, 'Tauri handler 注册表必须可解析');
const registered = new Set(mainRs.slice(handlerStart, handlerEnd).split('\n')
  .map(line => line.split('//')[0].trim().replace(/,$/, '')).filter(Boolean));
assert.ok(registered.has('task_board_change_status'), '状态拖放命令必须注册到 Tauri handler');
const wrapperStart = mainRs.indexOf('fn task_board_change_status(');
const wrapperEnd = mainRs.indexOf('\n#[tauri::command]', wrapperStart);
assert.ok(wrapperStart >= 0 && wrapperEnd > wrapperStart, '状态拖放命令 wrapper 必须可解析');
assert.match(mainRs.slice(wrapperStart, wrapperEnd), /s\.board_change_status\(/,
  'Tauri 命令必须转发到真实 Store');
assert.match(mainRs.slice(wrapperStart, wrapperEnd), /Ok\(data\) if data\.revision == expected_revision/,
  '同列 no-op 不得广播 tasks://changed');
assert.match(tasksRs, /status == RunStatus::Accepted[\s\S]{0,140}不能标记来源已验收/,
  '状态拖放不得将本地进度伪装成来源验收');

const server = createServer(async (req, res) => {
  try {
    const path = resolve(root, '.' + decodeURIComponent(new URL(req.url, 'http://localhost').pathname));
    if (!path.startsWith(root + '/app/ui/')) throw Error();
    res.writeHead(200, { 'content-type': ({ '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html' }[extname(path)] ?? 'application/octet-stream') });
    res.end(await readFile(path));
  } catch { res.writeHead(404); res.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true, ...(process.env.TEST_CHROMIUM_PATH ? { executablePath: process.env.TEST_CHROMIUM_PATH } : {}) });

try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 720 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    window.calls = [];
    window.boardState = {
      revision: 20,
      projects: [{ id: 'p-a', name: 'Alpha', dependencies: [] }],
      tasks: [
        { id: 't-running', title: '本地运行任务', project_id: 'p-a', source: null, current_run_id: 'r-running', archived_ms: null, revision: 1 },
        { id: 't-queued', title: '等待记录的任务', project_id: 'p-a', source: null, current_run_id: null, archived_ms: null, revision: 1 },
        { id: 't-finished', title: '已结束任务', project_id: 'p-a', source: null, current_run_id: 'r-ready', archived_ms: null, revision: 1 },
        { id: 't-archived', title: '归档任务', project_id: 'p-a', source: null, current_run_id: null, archived_ms: 5, revision: 2 },
      ],
      runs: [
        { id: 'r-running', task_id: 't-running', status: 'running', started_ms: 1, finished_ms: null, revision: 1 },
        { id: 'r-ready', task_id: 't-finished', status: 'ready', started_ms: 2, finished_ms: 3, revision: 2 },
      ],
      attentions: [], artifacts: [], event_receipts: [], evicted_artifacts: 0,
    };
    const copy = () => structuredClone(window.boardState);
    const columnFor = task => {
      const run = window.boardState.runs.find(item => item.id === task.current_run_id);
      if (!run) return 'queued';
      if (window.boardState.attentions.some(item => item.run_id === run.id && item.state === 'open') || ['waiting', 'failed'].includes(run.status)) return 'attention';
      return run.status === 'running' ? 'running' : run.status === 'queued' ? 'queued' : 'finished';
    };
    window.__TAURI__ = {
      core: { invoke: async (command, args = {}) => {
        window.calls.push({ command, args: structuredClone(args) });
        const state = window.boardState;
        if (command === 'get_boot_args') return { demo: false, expand: false, route: 'tasks' };
        if (command === 'get_settings') return { appearance: 'light', shell_mode: 'island', dock_edge: 'top', sample_interval: 2, idle_sample_interval: 5, disabled_agents: [], notification_policy: 'standard' };
        if (command === 'get_engine_state') return { snapshots: [], grand_total: { tokens24h: 0, tokens: 0, cost: 0 }, latest_event: null };
        if (command === 'window_is_visible') return true;
        if (command === 'drain_navigation') return [];
        if (command === 'tasks_attention_summary') return { revision: state.revision, total: 0, first: null };
        if (command === 'tasks_snapshot') return copy();
        if (command === 'task_sources') return [];
        if (command === 'task_board_change_status') {
          if (args.expectedRevision !== state.revision) throw { code: 'stale_revision', message: '任务已更新，请刷新后重试' };
          const task = state.tasks.find(item => item.id === args.id);
          if (!task) throw { code: 'not_found', message: '任务不存在' };
          if (task.archived_ms != null) throw { code: 'task_archived', message: '已归档任务不能拖动' };
          if (columnFor(task) !== args.sourceColumn) throw { code: 'source_column_mismatch', message: '任务所在看板列已变化，拖放未应用' };
          if (args.sourceColumn === args.targetColumn) return copy();
          const allowed = args.targetColumn === 'attention'
            ? (args.status === 'waiting' && ['answer', 'plan_approval', 'confirmation'].includes(args.kind))
              || (args.status === 'ready' && args.kind === 'result_review') || (args.status === 'failed' && !args.kind)
            : args.targetColumn === 'running' ? args.status === 'running' && !args.kind
              : args.targetColumn === 'queued' ? args.status === 'queued' && !args.kind
                : args.targetColumn === 'finished' ? ['ready', 'cancelled'].includes(args.status) && !args.kind : false;
          if (!allowed || args.status === 'accepted') throw { code: 'invalid_input', message: '所选本地进度与拖放目标不匹配' };
          if (window.failNextStatus) {
            const failure = window.failNextStatus;
            window.failNextStatus = null;
            throw failure;
          }
          const run = state.runs.find(item => item.id === task.current_run_id);
          if (!run || (['running', 'waiting'].includes(args.status) && ['ready', 'failed', 'cancelled', 'accepted'].includes(run.status))) {
            const next = { id: `r-${state.runs.length + 1}`, task_id: task.id, status: args.status, started_ms: 50, finished_ms: null, revision: 1 };
            state.runs.push(next); task.current_run_id = next.id;
          } else {
            run.status = args.status; run.revision++;
            run.finished_ms = ['ready', 'failed', 'cancelled', 'accepted'].includes(args.status) ? 50 : null;
          }
          for (const attention of state.attentions) if (attention.run_id === task.current_run_id && attention.state === 'open') attention.state = 'expired';
          if (args.kind) state.attentions.push({ id: `a-${state.attentions.length + 1}`, run_id: task.current_run_id, kind: args.kind, state: 'open', observed: false, artifact_id: null, manual_resolution: false });
          task.revision++; task.updated_ms = 50; state.revision++;
          return copy();
        }
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await page.goto(origin + '/app/ui/index.html?shell=workbench');
  await page.waitForSelector('[data-tasks-root][data-page-ready=true]');

  const statusCalls = () => page.evaluate(() => window.calls.filter(call => call.command === 'task_board_change_status'));
  const dropOnStatus = async (taskId, column, payload = taskId) => page.evaluate(({ taskId, column, payload }) => {
    const source = document.querySelector(`[data-task-select="${taskId}"]`);
    const target = document.querySelector(`[data-task-status-target="${column}"]`);
    const dataTransfer = new DataTransfer();
    source.dispatchEvent(new DragEvent('dragstart', { bubbles: true, cancelable: true, dataTransfer }));
    if (payload !== taskId) dataTransfer.setData('text/plain', payload);
    target.dispatchEvent(new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer }));
    target.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
    source.dispatchEvent(new DragEvent('dragend', { bubbles: true, dataTransfer }));
  }, { taskId, column, payload });

  assert.equal(await page.locator('[data-task-status-targets]').count(), 1, '状态拖放目标位于看板拖放栏');
  assert.equal(await page.locator('[data-task-project-targets]').isVisible(), false, '拖动前目标栏不占看板空间');
  await page.locator('[data-task-archived]').check();
  assert.equal(await page.locator('[data-task-select="t-archived"]').getAttribute('draggable'), 'false', '归档任务不可拖动');
  await page.locator('[data-task-archived]').uncheck();

  await dropOnStatus('t-running', 'running');
  assert.equal((await statusCalls()).length, 0, '同状态列是零写入 no-op');
  assert.equal(await page.locator('[data-task-status-dialog]').isVisible(), false, '同列 drop 不打开确认框');

  await dropOnStatus('t-running', 'finished', 'another-task');
  assert.equal((await statusCalls()).length, 0, '无效拖放 payload 不触发写入');
  assert.equal(await page.locator('[data-task-status-dialog]').isVisible(), false, '无效 payload 不打开确认框');

  const initialRevision = await page.evaluate(() => window.boardState.revision);
  await dropOnStatus('t-running', 'finished');
  const dialog = page.locator('[data-task-status-dialog]');
  assert.equal(await dialog.isVisible(), true, '状态 drop 只打开明确进度确认框');
  const choices = await page.locator('[data-task-status-select] option').allTextContents();
  assert.deepEqual(choices, ['选择具体进度', '结果就绪', '已取消'], '已结束列只提供本地终态选择');
  assert.ok(!choices.includes('来源已验收'), '拖放界面不提供来源验收选项');
  assert.equal((await statusCalls()).length, 0, '打开对话框本身不写入');
  await page.locator('[data-task-status-cancel]').click();
  assert.equal(await dialog.isVisible(), false, '取消关闭确认框');
  assert.equal((await statusCalls()).length, 0, '取消不写入');

  await dropOnStatus('t-running', 'finished');
  await page.selectOption('[data-task-status-select]', 'ready');
  await page.locator('[data-task-status-form] button[type="submit"]').click();
  await page.waitForTimeout(150);
  const submitState = await page.evaluate(() => ({
    calls: window.calls.filter(call => call.command === 'task_board_change_status'),
    feedback: document.querySelector('[data-task-status-feedback]')?.textContent,
    value: document.querySelector('[data-task-status-select]')?.value,
    valid: document.querySelector('[data-task-status-form]')?.checkValidity(),
    open: document.querySelector('[data-task-status-dialog]')?.open,
  }));
  assert.ok(submitState.calls.length, `确认提交状态拖放：${JSON.stringify(submitState)}`);
  await page.waitForFunction(() => window.boardState.runs.find(run => run.id === 'r-running')?.status === 'ready', null, { timeout: 5000 });
  const readyCall = (await statusCalls()).at(-1);
  assert.deepEqual(readyCall.args, {
    id: 't-running', sourceColumn: 'running', targetColumn: 'finished', status: 'ready', kind: null,
    expectedRevision: initialRevision,
  }, '确认时提交具体本地进度与 dragstart revision');
  assert.equal(await dialog.isVisible(), false, '保存成功后关闭确认框');
  assert.equal(await page.evaluate(() => window.boardState.runs.length), 2, '记录本地进度沿用当前运行，不重复创建运行');

  await dropOnStatus('t-running', 'attention');
  const attentionOptions = await page.locator('[data-task-status-select] option').allTextContents();
  assert.deepEqual(attentionOptions, ['选择具体进度', '等待处理 · 待回答', '等待处理 · 确认方案', '等待处理 · 验收结果', '等待处理 · 需要确认', '执行失败'],
    '需处理列要求选择具体关口或失败状态');
  await page.selectOption('[data-task-status-select]', 'waiting:plan_approval');
  await page.locator('[data-task-status-form] button[type="submit"]').click();
  await page.waitForFunction(() => window.boardState.attentions.length === 1);
  const attentionCall = (await statusCalls()).at(-1);
  assert.equal(attentionCall.args.status, 'waiting');
  assert.equal(attentionCall.args.kind, 'plan_approval');
  assert.equal(await page.evaluate(() => window.boardState.attentions[0].observed), false,
    '状态拖放创建的是本地关口记录，不伪装成来源观测');

  await dropOnStatus('t-queued', 'running');
  await page.selectOption('[data-task-status-select]', 'running');
  const beforeStale = (await statusCalls()).length;
  await page.evaluate(() => { window.boardState.revision++; });
  await page.locator('[data-task-status-form] button[type="submit"]').click();
  await page.waitForFunction(() => document.querySelector('[data-task-status-feedback]')?.textContent.includes('重新拖放'));
  assert.equal((await statusCalls()).length, beforeStale + 1, '过期请求只尝试一次');
  assert.equal(await page.locator('[data-task-status-select]').inputValue(), 'running', '过期拒绝保留用户所选进度');
  assert.equal(await dialog.isVisible(), true, '过期拒绝保留确认框以便用户检查反馈');
  assert.equal(await page.evaluate(() => window.boardState.tasks.find(task => task.id === 't-queued')?.current_run_id), null,
    '过期拖放不覆盖最新任务记录');
  await page.locator('[data-task-status-cancel]').click();
  await page.waitForFunction(() => !document.querySelector('[data-tasks-root]')?.hasAttribute('aria-busy'));

  await dropOnStatus('t-queued', 'running');
  await page.selectOption('[data-task-status-select]', 'running');
  await page.locator('[data-task-status-form] button[type="submit"]').click();
  await page.waitForFunction(() => {
    const task = window.boardState.tasks.find(item => item.id === 't-queued');
    return task?.current_run_id && window.boardState.runs.find(run => run.id === task.current_run_id)?.status === 'running';
  });
  const runningCall = (await statusCalls()).at(-1);
  assert.equal(runningCall.args.targetColumn, 'running');

  await dropOnStatus('t-queued', 'queued');
  await page.selectOption('[data-task-status-select]', 'queued');
  await page.locator('[data-task-status-form] button[type="submit"]').click();
  await page.waitForFunction(() => {
    const task = window.boardState.tasks.find(item => item.id === 't-queued');
    return window.boardState.runs.find(run => run.id === task?.current_run_id)?.status === 'queued';
  });
  const queuedCall = (await statusCalls()).at(-1);
  assert.equal(queuedCall.args.targetColumn, 'queued');

  await dropOnStatus('t-queued', 'running');
  await page.selectOption('[data-task-status-select]', 'running');
  const beforeWriteFailure = (await statusCalls()).length;
  await page.evaluate(() => { window.failNextStatus = { code: 'io_error', message: '任务保存失败，原记录已保留' }; });
  await page.locator('[data-task-status-form] button[type="submit"]').click();
  await page.waitForFunction(() => document.querySelector('[data-task-status-feedback]')?.textContent === '任务保存失败，原记录已保留');
  assert.equal((await statusCalls()).length, beforeWriteFailure + 1, '写入失败只尝试一次');
  assert.equal(await page.locator('[data-task-status-select]').inputValue(), 'running', '普通写入失败保留所选进度');
  assert.equal(await dialog.isVisible(), true, '普通写入失败保留确认框');
  assert.equal(await page.evaluate(() => {
    const task = window.boardState.tasks.find(item => item.id === 't-queued');
    return window.boardState.runs.find(run => run.id === task.current_run_id)?.status;
  }), 'queued', '写入失败保持任务原状态');
  await page.locator('[data-task-status-cancel]').click();

  // A detail draft remains authoritative and blocks the drag before opening the picker.
  await page.locator('[data-task-select="t-queued"]').click();
  await page.locator('[data-task-edit] input[name="title"]').fill('保留的任务草稿');
  const beforeDraft = (await statusCalls()).length;
  await dropOnStatus('t-queued', 'finished');
  assert.equal((await statusCalls()).length, beforeDraft, '任务详情草稿阻止状态拖放写入');
  assert.equal(await dialog.isVisible(), false, '草稿保护不打开状态确认框');
  assert.equal(await page.locator('[data-task-edit] input[name="title"]').inputValue(), '保留的任务草稿', '拒绝拖放保留详情草稿');

  assert.deepEqual(errors, [], `浏览器页面不能出现未处理异常：${errors.join('; ')}`);
  console.log('PASS task board status drag UI: explicit local progress confirmation, cancel/no-op/invalid payload, target choices, expected revision, stale preservation, and draft protection');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
