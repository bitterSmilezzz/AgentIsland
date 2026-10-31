import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, extname, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const { chromium } = await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const mainRs = await readFile(new URL('../app/src-tauri/src/main.rs', import.meta.url), 'utf8');
const handlerStart = mainRs.indexOf('tauri::generate_handler![');
const handlerEnd = mainRs.indexOf('])', handlerStart);
assert.ok(handlerStart >= 0 && handlerEnd > handlerStart, 'Tauri handler 注册表必须可解析');
const registeredCommands = new Set(mainRs.slice(handlerStart, handlerEnd).split('\n')
  .map(line => line.split('//')[0].trim().replace(/,$/, ''))
  .filter(Boolean).map(name => name.split('::').pop()));
for (const [command, storeMethod] of [
  ['task_project_dependencies_update', 'project_dependencies_update'],
  ['task_project_delete', 'project_delete'],
]) {
  assert.ok(registeredCommands.has(command), `${command} 必须注册到 Tauri handler`);
  const start = mainRs.indexOf(`fn ${command}(`);
  const end = mainRs.indexOf('\n#[tauri::command]', start);
  assert.ok(start >= 0 && end > start, `${command} wrapper 必须可解析`);
  assert.match(mainRs.slice(start, end), new RegExp(`s\\.${storeMethod}\\(`),
    `${command} 必须转发到 Store::${storeMethod}`);
}
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
    window.projectState = {
      revision: 1,
      projects: [{ id: 'p-a', name: '前端', dependencies: [] }, { id: 'p-b', name: '接口', dependencies: [] }],
      tasks: [], runs: [], attentions: [], artifacts: [],
    };
    window.nextDependencyFailure = null;
    window.__TAURI__ = {
      core: { invoke: async (command, args) => {
        window.calls.push({ command, args });
        if (command === 'get_boot_args') return { demo: false, expand: false, route: 'tasks' };
        if (command === 'get_settings') return { appearance: 'light', shell_mode: 'island', dock_edge: 'top', sample_interval: 2, idle_sample_interval: 5, disabled_agents: [], notification_policy: 'standard' };
        if (command === 'get_engine_state') return { snapshots: [], grand_total: { tokens24h: 0, tokens: 0, cost: 0 }, latest_event: null };
        if (command === 'window_is_visible') return true;
        if (command === 'drain_navigation') return [];
        if (command === 'tasks_attention_summary') return { revision: 1, total: 0, first: null };
        if (command === 'tasks_snapshot') return structuredClone(window.projectState);
        if (command === 'task_sources') return [];
        if (command === 'task_project_dependencies_update') {
          if (window.nextDependencyFailure === 'stale_revision') {
            window.nextDependencyFailure = null;
            window.projectState.revision++;
            throw { code: 'stale_revision', message: '任务已更新，请刷新后重试' };
          }
          if (window.nextDependencyFailure) {
            const code = window.nextDependencyFailure;
            window.nextDependencyFailure = null;
            throw { code, message: code === 'dependency_cycle' ? '该依赖会形成循环' : '拒绝该依赖' };
          }
          window.projectState.revision++;
          window.projectState.projects = window.projectState.projects.map(project => project.id === args.projectId
            ? { ...project, dependencies: args.dependencies } : project);
          return structuredClone(window.projectState);
        }
        if (command === 'task_project_delete') throw { code: 'still_depended_on', message: '仍有其他项目依赖此项目' };
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await page.goto(origin + '/app/ui/index.html?shell=workbench');
  await page.waitForSelector('[data-tasks-root][data-page-ready=true]');
  await page.locator('.task-project-create summary').click();

  const projectSelect = page.locator('[data-task-project-select]');
  const savedProjects = await page.evaluate(() => structuredClone(window.projectState.projects));
  await page.evaluate(() => { window.projectState.projects = []; });
  await page.evaluate(async () => await document.querySelector('[data-tasks-root]').refreshTasks());
  assert.equal(await page.locator('[data-task-project-delete]').evaluate(element => getComputedStyle(element).display), 'none',
    'delete control stays hidden when there are no projects');
  assert.equal(await page.locator('[data-task-project-dependencies]').evaluate(element => getComputedStyle(element).display), 'none',
    'dependency editor stays hidden when there is no selected project');
  assert.equal(await page.locator('[data-task-project-dependencies]').evaluate(element => element.onsubmit), null,
    'empty project state clears the old dependency submit handler');
  await page.evaluate(projects => { window.projectState.projects = projects.slice(0, 1); }, savedProjects);
  await page.evaluate(async () => await document.querySelector('[data-tasks-root]').refreshTasks());
  assert.equal(await page.locator('[data-task-project-dependencies]').evaluate(element => element.onsubmit), null,
    'single project state clears the old dependency submit handler');
  await page.evaluate(projects => { window.projectState.projects = projects; }, savedProjects);
  await page.evaluate(async () => await document.querySelector('[data-tasks-root]').refreshTasks());
  await page.selectOption('[data-task-project-select]', 'p-a');
  await page.locator('[data-project-dependency="p-b"]').check();
  await page.locator('[data-task-project] input[name="name"]').fill('新项目');
  await page.locator('[data-task-project] button[type="submit"]').click();
  await page.waitForFunction(() => document.querySelector('[data-task-feedback]')?.textContent.includes('依赖草稿'));
  assert.equal(await page.evaluate(() => window.projectState.projects.length), 2, 'project creation must not invalidate an unsaved dependency draft');
  assert.equal(await page.locator('[data-project-dependency="p-b"]').isChecked(), true, 'blocked project creation preserves dependency draft');
  assert.equal(await page.evaluate(() => window.calls.some(call => call.command === 'task_project_create')), false, 'blocked project creation must not advance the shared revision');
  await page.locator('[data-task-project-dependencies] button[type="submit"]').click();
  await page.waitForFunction(() => window.calls.some(call => call.command === 'task_project_dependencies_update'));
  const addCall = await page.evaluate(() => window.calls.findLast(call => call.command === 'task_project_dependencies_update'));
  assert.deepEqual(addCall.args.dependencies, ['p-b']);
  assert.equal(addCall.args.expectedRevision, 1);
  assert.equal(await page.locator('[data-project-dependency="p-b"]').isChecked(), true, 'saved dependency stays visible');

  await page.locator('[data-project-dependency="p-b"]').uncheck();
  await page.locator('[data-task-project-dependencies] button[type="submit"]').click();
  await page.waitForFunction(() => window.projectState.revision === 3);
  assert.deepEqual(await page.evaluate(() => window.projectState.projects[0].dependencies), [], 'removing a dependency persists');

  await page.selectOption('[data-task-project-select]', 'p-a');
  await page.locator('[data-project-dependency="p-b"]').check();
  await page.locator('[data-task-project-dependencies] button[type="submit"]').click();
  await page.waitForFunction(() => window.projectState.revision === 4);
  await page.selectOption('[data-task-project-select]', 'p-b');
  await page.locator('[data-project-dependency="p-a"]').check();
  await page.evaluate(() => { window.nextDependencyFailure = 'dependency_cycle'; });
  await page.locator('[data-task-project-dependencies] button[type="submit"]').click();
  await page.waitForFunction(() => document.querySelector('[data-task-project-dependencies] [data-task-form-feedback]')?.textContent.includes('循环'));
  assert.equal(await page.locator('[data-project-dependency="p-a"]').isChecked(), true, 'rejected dependency remains an editable draft');
  await page.locator('[data-task-discard]').click();
  await page.locator('[data-project-dependency="p-a"]').check();
  await page.evaluate(() => { window.nextDependencyFailure = 'stale_revision'; });
  await page.locator('[data-task-project-dependencies] button[type="submit"]').click();
  await page.waitForFunction(() => document.querySelector('[data-task-project-dependencies] [data-task-form-feedback]')?.textContent.includes('草稿已保留'));
  await page.evaluate(async () => await document.querySelector('[data-tasks-root]').refreshTasks());
  assert.equal(await page.locator('[data-task-project-dependencies]').getAttribute('data-base-revision'), '4', 'refresh leaves the stale draft bound to its original revision');
  assert.equal(await page.locator('[data-project-dependency="p-a"]').isChecked(), true, 'refresh after a conflict must preserve the draft');
  await page.locator('[data-task-discard]').click();
  assert.equal(await page.locator('[data-project-dependency="p-a"]').isChecked(), false, 'discard reloads the current saved dependencies');
  assert.equal(await page.locator('[data-task-project-dependencies]').getAttribute('data-base-revision'), '5', 'discard rebases on the refreshed revision');

  await page.locator('[data-task-project-delete-button]').click();
  await page.waitForFunction(() => document.querySelector('[data-task-feedback]')?.textContent.includes('仍有其他项目依赖'));
  assert.ok(await projectSelect.count());
  assert.deepEqual(errors, []);
  await page.close();
  console.log('PASS: project dependencies add/remove, refusal feedback, stale draft retention and delete guard');
} finally {
  await browser.close();
  server.close();
}
