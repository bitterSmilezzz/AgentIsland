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
assert.ok(registered.has('task_board_move_project'), '拖放命令必须注册到 Tauri handler');
const wrapperStart = mainRs.indexOf('fn task_board_move_project(');
const wrapperEnd = mainRs.indexOf('\n#[tauri::command]', wrapperStart);
assert.ok(wrapperStart >= 0 && wrapperEnd > wrapperStart, '拖放命令 wrapper 必须可解析');
assert.match(mainRs.slice(wrapperStart, wrapperEnd), /s\.board_move_project\(/,
  'Tauri 命令必须转发到真实 Store');
assert.match(mainRs.slice(wrapperStart, wrapperEnd), /Ok\(data\) if data\.revision == expected_revision/,
  '同项目 no-op 不得广播 tasks://changed');
assert.match(tasksRs, /resolve_drag\(/, 'Store 写入路径必须消费拖拽判据');

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
      revision: 10,
      projects: [{ id: 'p-a', name: 'Alpha前端', dependencies: [] }, { id: 'p-b', name: 'Beta接口', dependencies: [] }],
      tasks: [
        { id: 't-active', title: '运行任务', project_id: 'p-a', source: null, current_run_id: 'r-running', archived_ms: null, revision: 1 },
        { id: 't-other', title: '普通任务', project_id: 'p-a', source: null, current_run_id: null, archived_ms: null, revision: 1 },
        { id: 't-archived', title: '归档任务', project_id: 'p-a', source: null, current_run_id: null, archived_ms: 5, revision: 2 },
      ],
      runs: [{ id: 'r-running', task_id: 't-active', status: 'running', started_ms: 1, finished_ms: null, revision: 1 }],
      attentions: [], artifacts: [], event_receipts: [], evicted_artifacts: 0,
    };
    window.__TAURI__ = {
      core: { invoke: async (command, args = {}) => {
        window.calls.push({ command, args: structuredClone(args) });
        const state = window.boardState;
        const copy = () => structuredClone(state);
        if (command === 'get_boot_args') return { demo: false, expand: false, route: 'tasks' };
        if (command === 'get_settings') return { appearance: 'light', shell_mode: 'island', dock_edge: 'top', sample_interval: 2, idle_sample_interval: 5, disabled_agents: [], notification_policy: 'standard' };
        if (command === 'get_engine_state') return { snapshots: [], grand_total: { tokens24h: 0, tokens: 0, cost: 0 }, latest_event: null };
        if (command === 'window_is_visible') return true;
        if (command === 'drain_navigation') return [];
        if (command === 'tasks_attention_summary') return { revision: state.revision, total: 0, first: null };
        if (command === 'tasks_snapshot') return copy();
        if (command === 'task_sources') return [];
        if (command === 'task_board_move_project') {
          if (args.expectedRevision !== state.revision) throw { code: 'stale_revision', message: '任务已更新，请刷新后重试' };
          const task = state.tasks.find(item => item.id === args.id);
          if (!task) throw { code: 'not_found', message: '任务不存在' };
          if (task.archived_ms != null) throw { code: 'task_archived', message: '已归档任务不能拖动' };
          if (task.project_id !== null && !state.projects.some(item => item.id === task.project_id)) throw { code: 'target_not_found', message: '目标项目不存在' };
          const run = state.runs.find(item => item.id === task.current_run_id);
          const currentColumn = !run ? 'queued' : state.attentions.some(item => item.run_id === run.id && item.state === 'open') || ['waiting', 'failed'].includes(run.status)
            ? 'attention' : run.status === 'running' ? 'running' : run.status === 'queued' ? 'queued' : 'finished';
          if (args.sourceColumn !== currentColumn) throw { code: 'source_column_mismatch', message: '任务所在看板列已变化，拖放未应用' };
          if (args.projectId != null && !state.projects.some(item => item.id === args.projectId)) throw { code: 'target_not_found', message: '目标项目不存在' };
          if (task.project_id === (args.projectId ?? null)) return copy();
          task.project_id = args.projectId ?? null;
          task.revision++;
          task.updated_ms = 50;
          state.revision++;
          return copy();
        }
        if (command === 'task_update') {
          if (args.expectedRevision !== state.revision) throw { code: 'stale_revision', message: '任务已更新，请刷新后重试' };
          const task = state.tasks.find(item => item.id === args.id);
          if (!task) throw { code: 'not_found', message: '任务不存在' };
          task.title = args.title;
          task.project_id = args.projectId ?? null;
          task.revision++;
          state.revision++;
          return copy();
        }
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await page.goto(origin + '/app/ui/index.html?shell=workbench');
  await page.waitForSelector('[data-tasks-root][data-page-ready=true]');

  const moveCalls = () => page.evaluate(() => window.calls.filter(call => call.command === 'task_board_move_project'));
  const initialRevision = await page.evaluate(() => window.boardState.revision);
  const runningCard = page.locator('[data-task-select="t-active"]');
  const interfaceTarget = page.locator('[data-task-project-target="p-b"]');
  assert.equal(await runningCard.getAttribute('draggable'), 'true', '未归档任务卡可以拖动');
  assert.equal(await page.locator('[data-task-project-targets]').isVisible(), false, '拖动开始前目标栏不占用看板空间');
  const dragToProject = async (card, target) => {
    const box = await card.boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 12, box.y + box.height / 2 + 8, { steps: 2 });
    await page.waitForFunction(() => !document.querySelector('[data-task-project-targets]').hidden,
      null, { timeout: 3000 });
    assert.equal(await target.isVisible(), true, '拖动开始后显示项目目标');
    const targetBox = await target.boundingBox();
    await page.mouse.move(targetBox.x + targetBox.width / 2, targetBox.y + targetBox.height / 2, { steps: 5 });
    await page.mouse.up();
  };
  await dragToProject(runningCard, interfaceTarget);
  await page.waitForFunction(() => window.boardState.tasks.find(task => task.id === 't-active')?.project_id === 'p-b');
  const firstMove = (await moveCalls())[0];
  assert.equal(firstMove.args.expectedRevision, initialRevision, '拖动使用开始时捕获的聚合 revision');
  assert.equal(firstMove.args.sourceColumn, 'running', '拖动带出来源列');
  assert.equal(firstMove.args.projectId, 'p-b');
  assert.equal(await page.evaluate(() => window.boardState.runs.length), 1, '改变项目归属不新增运行记录');
  assert.equal(await page.evaluate(() => window.boardState.runs[0].status), 'running', '改变项目归属不改变任务状态');

  const noOpRevision = await page.evaluate(() => window.boardState.revision);
  await dragToProject(page.locator('[data-task-select="t-active"]'), interfaceTarget);
  await page.waitForFunction(() => window.calls.filter(call => call.command === 'task_board_move_project').length === 2);
  assert.equal(await page.evaluate(() => window.boardState.revision), noOpRevision, '同项目拖放不推进 revision');
  assert.equal(await page.evaluate(() => window.boardState.tasks[0].revision), 2, '同项目拖放不推进任务 revision');
  assert.equal(await page.locator('[data-task-project-targets]').isVisible(), false, '正常 drop 后目标栏收起');

  // Keyboard path: use native-select typeahead, then Tab to Save and press Enter.
  await page.locator('[data-task-select="t-active"]').click();
  await page.waitForSelector('[data-task-edit]');
  const detailProject = page.locator('[data-task-edit] select[name="projectId"]');
  await detailProject.focus();
  await page.keyboard.type('Alpha');
  assert.equal(await detailProject.inputValue(), 'p-a', '键盘输入项目名可选中项目');
  await page.keyboard.press('Tab'); // newly revealed "撤销修改" draft action
  await page.keyboard.press('Tab'); // Save
  assert.equal(await page.evaluate(() => document.activeElement.textContent), '保存', 'Tab should reach the Save button after the draft action appears');
  await page.keyboard.press('Enter');
  await page.waitForFunction(() => window.boardState.tasks.find(task => task.id === 't-active')?.project_id === 'p-a');
  assert.ok(await page.locator('[data-task-feedback]').textContent().then(text => text.includes('已保存')),
    '键盘可通过任务详情的项目选择完成归属修改');

  const projectSummary = page.locator('.task-project-create summary');
  await projectSummary.click();
  await page.selectOption('[data-task-project-select]', 'p-a');
  await page.locator('[data-project-dependency="p-b"]').check();
  const beforeDraftDrop = (await moveCalls()).length;
  await page.evaluate(() => {
    const source = document.querySelector('[data-task-select="t-other"]');
    const target = document.querySelector('[data-task-project-target="p-b"]');
    const dataTransfer = new DataTransfer();
    source.dispatchEvent(new DragEvent('dragstart', { bubbles: true, cancelable: true, dataTransfer }));
    target.dispatchEvent(new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer }));
    target.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
    source.dispatchEvent(new DragEvent('dragend', { bubbles: true, dataTransfer }));
  });
  assert.equal((await moveCalls()).length, beforeDraftDrop, '未保存项目依赖草稿阻止拖放写入');
  assert.equal(await page.locator('[data-project-dependency="p-b"]').isChecked(), true, '拒绝拖放保留项目依赖草稿');
  assert.equal(await page.locator('[data-task-project-targets]').isVisible(), false, '草稿保护拒绝后目标栏收起');
  const dependencyFeedback = await page.locator('[data-task-feedback]').textContent();
  assert.ok(dependencyFeedback.includes('未保存草稿'), `项目依赖草稿拒绝反馈：${dependencyFeedback}`);
  await page.locator('[data-task-project-dependencies] [data-task-discard]').click();

  // An unrelated detail draft also blocks moving another card.
  await page.locator('[data-task-edit] input[name="title"]').fill('尚未保存的标题');
  const beforeDetailDraftDrop = (await moveCalls()).length;
  await page.evaluate(() => {
    const source = document.querySelector('[data-task-select="t-other"]');
    const target = document.querySelector('[data-task-project-target="p-b"]');
    const dataTransfer = new DataTransfer();
    source.dispatchEvent(new DragEvent('dragstart', { bubbles: true, cancelable: true, dataTransfer }));
    target.dispatchEvent(new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer }));
    target.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
    source.dispatchEvent(new DragEvent('dragend', { bubbles: true, dataTransfer }));
  });
  assert.equal((await moveCalls()).length, beforeDetailDraftDrop, '未保存任务详情草稿阻止拖放写入');
  assert.equal(await page.locator('[data-task-edit] input[name="title"]').inputValue(), '尚未保存的标题', '拒绝拖放保留任务详情草稿');
  assert.equal(await page.locator('[data-task-project-targets]').isVisible(), false, '详情草稿保护拒绝后目标栏收起');
  await page.locator('[data-task-edit] [data-task-discard]').click();

  const beforeMismatchedDrop = (await moveCalls()).length;
  await page.evaluate(() => {
    const source = document.querySelector('[data-task-select="t-other"]');
    const target = document.querySelector('[data-task-project-target="p-b"]');
    const dataTransfer = new DataTransfer();
    source.dispatchEvent(new DragEvent('dragstart', { bubbles: true, cancelable: true, dataTransfer }));
    dataTransfer.setData('text/plain', 'another-task');
    target.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
    source.dispatchEvent(new DragEvent('dragend', { bubbles: true, dataTransfer }));
  });
  assert.equal((await moveCalls()).length, beforeMismatchedDrop, '拖放数据不匹配时不写入');
  assert.equal(await page.locator('[data-task-project-targets]').isVisible(), false, '无效 drop 后目标栏收起');
  assert.ok((await page.locator('[data-task-feedback]').textContent()).includes('拖动任务已失效'), '无效 drop 显示失效反馈');

  await page.locator('[data-task-archived]').check();
  assert.equal(await page.locator('[data-task-select="t-archived"]').getAttribute('draggable'), 'false', '归档任务卡不提供拖动');
  assert.equal(await page.locator('[data-task-project-targets]').isVisible(), false, '归档列表不显示可写入的拖放目标');
  await page.locator('[data-task-archived]').uncheck();

  // A refresh requested during the drag is queued. The submitted revision stays
  // bound to dragstart and the stale write is rejected before the new snapshot loads.
  const beforeStaleDrop = (await moveCalls()).length;
  await page.evaluate(async () => {
    const source = document.querySelector('[data-task-select="t-other"]');
    const target = document.querySelector('[data-task-project-target="p-b"]');
    const dataTransfer = new DataTransfer();
    source.dispatchEvent(new DragEvent('dragstart', { bubbles: true, cancelable: true, dataTransfer }));
    window.boardState.revision++;
    await document.querySelector('[data-tasks-root]').refreshTasks();
    target.dispatchEvent(new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer }));
    target.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
    source.dispatchEvent(new DragEvent('dragend', { bubbles: true, dataTransfer }));
  });
  await page.waitForFunction(() => document.querySelector('[data-task-feedback]')?.textContent.includes('拖放未应用'));
  const staleCall = (await moveCalls()).at(-1);
  assert.ok(staleCall.args.expectedRevision < await page.evaluate(() => window.boardState.revision),
    '拖放请求保留 dragstart 时的旧 revision');
  assert.equal(await page.evaluate(() => window.boardState.tasks.find(task => task.id === 't-other')?.project_id), 'p-a',
    '过期拖放不会覆盖服务端最新归属');

  const callsAfterStale = (await moveCalls()).length;
  const revisionAfterStale = await page.evaluate(() => window.boardState.revision);
  await page.evaluate(() => {
    const source = document.querySelector('[data-task-select="t-other"]');
    const dataTransfer = new DataTransfer();
    source.dispatchEvent(new DragEvent('dragstart', { bubbles: true, cancelable: true, dataTransfer }));
    source.dispatchEvent(new DragEvent('dragend', { bubbles: true, dataTransfer }));
  });
  assert.equal((await moveCalls()).length, callsAfterStale, '取消拖动不提交写入');
  assert.equal(await page.evaluate(() => window.boardState.revision), revisionAfterStale, '取消拖动不推进 revision');

  assert.deepEqual(errors, []);
  await page.close();
  console.log('PASS: project drag, keyboard path, no-op, draft guards, archived tasks and stale revision');
} finally {
  await browser.close();
  server.close();
}
