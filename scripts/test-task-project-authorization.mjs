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
assert.ok(handlerStart >= 0 && handlerEnd > handlerStart, 'Tauri handler registration must be parseable');
const registered = new Set(mainRs.slice(handlerStart, handlerEnd).split('\n')
  .map(line => line.split('//')[0].trim().replace(/,$/, '')).filter(Boolean).map(name => name.split('::').pop()));
for (const command of [
  'task_project_authorization_snapshot',
  'task_project_authorization_choose_root',
  'task_project_authorization_cancel_consent',
  'task_project_authorization_confirm',
  'task_project_authorization_set_history',
  'task_project_authorization_revoke',
]) {
  assert.ok(registered.has(command), `${command} must be registered with Tauri`);
  assert.ok(mainRs.includes(`fn ${command}(`), `${command} must have a command implementation`);
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
    window.confirmPrompts = [];
    window.confirm = message => { window.confirmPrompts.push(message); return true; };
    window.projectState = {
      revision: 1,
      projects: [{ id: '00000000-0000-4000-8000-000000000001', name: '本地项目', dependencies: [] }],
      tasks: [], runs: [], attentions: [], artifacts: [],
    };
    window.authorization = {
      projectId: window.projectState.projects[0].id,
      policyRevision: 0,
      status: 'not_authorized',
      rootPath: null,
      roles: [],
      historyCaptureEnabled: false,
    };
    window.nextCandidate = null;
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
        if (command === 'task_project_authorization_snapshot') return structuredClone(window.authorization);
        if (command === 'task_project_authorization_choose_root') {
          const candidate = window.nextCandidate;
          window.nextCandidate = null;
          return candidate ? structuredClone(candidate) : null;
        }
        if (command === 'task_project_authorization_cancel_consent') return null;
        if (command === 'task_project_authorization_confirm') {
          window.authorization = {
            projectId: args.projectId,
            policyRevision: window.authorization.policyRevision + 1,
            status: 'authorized',
            rootPath: '/Users/test/本地项目',
            roles: args.roles,
            historyCaptureEnabled: args.historyCaptureEnabled,
          };
          return structuredClone(window.authorization);
        }
        if (command === 'task_project_authorization_set_history') {
          window.authorization.policyRevision++;
          window.authorization.historyCaptureEnabled = args.enabled;
          return structuredClone(window.authorization);
        }
        if (command === 'task_project_authorization_revoke') {
          window.authorization.policyRevision++;
          window.authorization.status = 'not_authorized';
          window.authorization.rootPath = null;
          window.authorization.roles = [];
          window.authorization.historyCaptureEnabled = false;
          return structuredClone(window.authorization);
        }
        return null;
      } },
      event: { listen: async () => () => {} },
    };
  });
  await page.goto(origin + '/app/ui/index.html?shell=workbench');
  await page.waitForSelector('[data-tasks-root][data-page-ready=true]');
  await page.locator('.task-project-create summary').click();
  const panel = page.locator('[data-task-project-authorization]');
  await page.waitForFunction(() => document.querySelector('[data-task-project-authorization]')?.textContent.includes('未授权'));

  await panel.locator('[data-task-authorization-edit]').click();
  await page.waitForFunction(() => window.calls.some(call => call.command === 'task_project_authorization_choose_root'));
  assert.equal(await page.evaluate(() => window.calls.filter(call => call.command === 'task_project_authorization_confirm').length), 0,
    'cancelled native folder picking must not write a grant');
  assert.equal(await page.evaluate(() => window.authorization.policyRevision), 0);

  const candidate = { token: 'candidate-one', projectId: '00000000-0000-4000-8000-000000000001', canonicalPath: '/Users/test/本地项目', expectedPolicyRevision: 0 };
  await page.evaluate(value => { window.nextCandidate = value; }, candidate);
  await panel.locator('[data-task-authorization-edit]').click();
  await page.waitForSelector('[data-task-authorization-confirm]');
  assert.equal(await panel.locator('.task-project-root-path').textContent(), '/Users/test/本地项目', 'confirmation shows the canonical selected root');
  assert.equal(await panel.locator('[data-task-authorization-history]').isChecked(), false, 'history capture defaults off');
  assert.equal(await panel.locator('[data-task-authorization-confirm]').isDisabled(), true, 'no role is preselected');
  await panel.locator('[data-task-authorization-role="planner"]').check();
  await panel.locator('[data-task-authorization-role="reviewer"]').check();
  await panel.locator('[data-task-authorization-cancel]').click();
  await page.waitForFunction(() => document.querySelector('[data-task-project-authorization]')?.textContent.includes('未授权'));
  assert.ok(await page.evaluate(() => window.calls.some(call => call.command === 'task_project_authorization_cancel_consent')),
    'cancelled consent invalidates the one-use candidate');
  assert.equal(await page.evaluate(() => window.calls.filter(call => call.command === 'task_project_authorization_confirm').length), 0,
    'cancel after preview does not write a grant');

  await page.evaluate(value => { window.nextCandidate = { ...value, token: 'candidate-two' }; }, candidate);
  await panel.locator('[data-task-authorization-edit]').click();
  await panel.locator('[data-task-authorization-role="planner"]').check();
  await panel.locator('[data-task-authorization-role="reviewer"]').check();
  await panel.locator('[data-task-authorization-confirm]').click();
  await page.waitForFunction(() => window.authorization.status === 'authorized');
  const grant = await page.evaluate(() => window.calls.findLast(call => call.command === 'task_project_authorization_confirm'));
  assert.deepEqual(grant.args.roles, ['planner', 'reviewer']);
  assert.equal(grant.args.historyCaptureEnabled, false);
  assert.equal(grant.args.expectedPolicyRevision, 0);
  assert.equal(grant.args.candidateToken, 'candidate-two');

  await panel.locator('[data-task-authorization-history]').check();
  await panel.locator('[data-task-authorization-save-history]').click();
  await page.waitForFunction(() => window.authorization.historyCaptureEnabled === true);
  const history = await page.evaluate(() => window.calls.findLast(call => call.command === 'task_project_authorization_set_history'));
  assert.equal(history.args.enabled, true);
  assert.equal(await page.evaluate(() => window.confirmPrompts.length), 1, 'enabling history requires a second explicit confirmation');

  await panel.locator('[data-task-authorization-revoke]').click();
  await page.waitForFunction(() => window.authorization.status === 'not_authorized');
  const revoke = await page.evaluate(() => window.calls.findLast(call => call.command === 'task_project_authorization_revoke'));
  assert.equal(revoke.args.expectedPolicyRevision, 2);
  assert.equal(await page.evaluate(() => window.confirmPrompts.length), 2, 'revocation requires explicit confirmation');
  assert.equal(await page.evaluate(() => window.calls.some(call => /run|execute/i.test(call.command))), false,
    'consent UI must not start a runner or an external agent');
  assert.deepEqual(errors, []);
  await page.close();
  console.log('PASS: project consent preview/cancel, default-off history, role grant, explicit history opt-in and revoke');
} finally {
  await browser.close();
  server.close();
}
