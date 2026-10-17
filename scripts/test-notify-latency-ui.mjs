import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, extname, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
const { chromium } = await import(process.env.TEST_BROWSER_MODULE || 'playwright');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
// 键名契约夹具由 Rust 侧 `engine_state_notify_keys_are_snake_case_contract` 读同一份文件。
// 两侧各自写一份字面量的话，可以被「一致地」改成一个新名字而 views.js 没跟上：
// 那时 Rust 测试与这条浏览器测试都绿，真机上通知页却全部显示「未知」。
const contract = JSON.parse(await readFile(
  new URL('../app/src-tauri/tests/fixtures/contracts/notify-keys.json', import.meta.url), 'utf8'));
const viewsSource = await readFile(new URL('../app/ui/js/views.js', import.meta.url), 'utf8');
// **不变量：界面调用的每个 Tauri 命令都必须在 `invoke_handler` 里注册。**
// 此前无人守卫这条。而界面侧常用 `.catch(() => …)` 兜底，所以「命令没注册」
// 不会报错——只会静默降级（例如误删 `remote_escalations` 那一行注册，
// 故障告警条永远不出现，却看不出任何异常）。这正是 A2 那类「实现了没接线」的形态。
// 只做**单向**检查：界面调用 ⊄ 注册表 是缺陷；注册了没人调是另一回事（无害）。
const mainRs = await readFile(new URL('../app/src-tauri/src/main.rs', import.meta.url), 'utf8');
const handlerBlock = mainRs.slice(
  mainRs.indexOf('tauri::generate_handler!['),
  mainRs.indexOf('])', mainRs.indexOf('tauri::generate_handler![')));
const registered = new Set(
  handlerBlock.split('\n')
    .map(line => line.split('//')[0].trim().replace(/,$/, ''))
    .filter(Boolean)
    .map(name => name.split('::').pop()));
const uiDir = new URL('../app/ui/js/', import.meta.url);
const { readdir } = await import('node:fs/promises');
const invoked = new Set();
for (const file of (await readdir(uiDir)).filter(f => f.endsWith('.js'))) {
  const source = await readFile(new URL(file, uiDir), 'utf8');
  for (const match of source.matchAll(/invoke\(\s*'([a-z0-9_]+)'/g)) invoked.add(match[1]);
}
assert.ok(registered.size > 50, `注册表解析异常（只读到 ${registered.size} 个命令）——守卫自身失效`);
const unregistered = [...invoked].filter(cmd => !registered.has(cmd)).sort();
assert.deepEqual(unregistered, [],
  `界面调用了未注册的 Tauri 命令：${unregistered.join(', ')}——命令没接线时界面会静默降级`);
const server = createServer(async (req, res) => {
  try {
    const path = resolve(root, '.' + decodeURIComponent(new URL(req.url, 'http://localhost').pathname));
    if (!path.startsWith(root + '/app/ui/')) throw Error();
    res.writeHead(200, { 'content-type': ({ '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.svg': 'image/svg+xml' })[extname(path)] || 'application/octet-stream' });
    res.end(await readFile(path));
  } catch { res.writeHead(404); res.end(); }
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.TEST_CHROMIUM_PATH ? { executablePath: process.env.TEST_CHROMIUM_PATH } : {}) });
  const page = await browser.newPage({ viewport: { width: 1180, height: 800 } });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.addInitScript(() => {
    window.calls = [];
    window.latency = { samples: 0, p95Ms: null, discarded: 0, verdictAvailable: false, meetsTarget: null };
    window.diagnostics = {};
    window.engineFixture = () => ({ snapshots: [], grand_total: { tokens24h: 0 }, latest_event: null, notify_latency: window.latency, ...window.diagnostics });
    window.__TAURI__ = { core: { invoke: async (cmd) => {
      window.calls.push(cmd);
      if (cmd === 'get_boot_args') return { demo: false, route: 'remote' };
      if (cmd === 'get_settings') return { appearance: 'light', dock_edge: 'top', shell_mode: 'island', disabled_agents: [], remote_kind: 'smtpEmail', remote_channels: { smtpEmail: {} }, remote_policy: { master_enabled: false } };
      if (cmd === 'get_engine_state') {
        if (window.latencyReadFails) throw Error('fixture read failure');
        if (window.delayLatency) {
          const snapshot = window.engineFixture();
          return new Promise(r => { window.finishLatency = () => r(snapshot); });
        }
        if (window.latencyDelayMs) await new Promise(r => setTimeout(r, window.latencyDelayMs));
        return window.engineFixture();
      }
      if (cmd === 'remote_status') return { kind: 'smtpEmail', secretName: 'remote.smtpEmail', policy: { master_enabled: false }, limitations: '' };
      // 夹具放 sessionStorage：page.reload() 会清空 window 状态，而测试需要
      // 「换夹具 → 重载 → 再断言」。initScript 不能移除，sessionStorage 可增可删。
      if (cmd === 'remote_recent') {
        const raw = window.sessionStorage.getItem('recentFixture');
        return raw ? JSON.parse(raw) : [];
      }
      if (cmd === 'remote_escalations') {
        const raw = window.sessionStorage.getItem('escalationsFixture');
        return raw ? JSON.parse(raw) : [];
      }
      if (cmd === 'window_is_visible') return true;
      if (cmd === 'drain_navigation') return [];
      if (cmd === 'tasks_attention_summary') return { revision: 0, total: 0, first: null };
      return null;
    } }, event: { listen: async () => () => {} } };
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/app/ui/index.html?shell=workbench`);
  await page.waitForSelector('[data-remote-root][data-page-ready=true]');
  // Locate the product diagnostic by its visible heading, without assuming its exact DOM shape.
  const group = page.locator('.sb-group').filter({ hasText: '本地事件入队延迟' });
  await assert.doesNotReject(() => group.waitFor());
  // 夹具契约：views.js 必须引用夹具里列的每一个引擎键，且不得引用夹具外的 notify_* 键。
  // 这条把「Rust 出口名」与「界面读取名」钉在同一份来源上，任何单侧改名都会让测试变红。
  for (const key of contract.engine_state) {
    assert.ok(viewsSource.includes(key), `views.js 未引用契约键 ${key}`);
  }
  for (const match of viewsSource.matchAll(/\bnotify_[a-z_]+/g)) {
    assert.ok(contract.engine_state.includes(match[0]),
      `views.js 引用了契约外的键 ${match[0]}——Rust 出口里没有它，真机上是 undefined`);
  }
  // 事件类型也必须对着同一份清单：每类都要在界面里有专属文案，
  // 否则新类型会掉进兜底分支、被显示成「任务完成」——那等于类型白加了。
  for (const kind of contract.event_kinds) {
    assert.ok(viewsSource.includes(`'${kind}'`),
      `views.js 未处理事件类型 ${kind}——它会掉进兜底文案`);
  }
  // 逐类文案必须互不相同：同一个字符串套在所有类型上，界面就分不出来。
  // `completed` 允许走 `default` 兜底（它是唯一以「完成」为语义的默认值），
  // 其余类型必须有显式分支——否则它们只能落到 default 上被说成「任务完成」。
  const summaries = new Set();
  for (const kind of contract.event_kinds) {
    const re = new RegExp(`(?:case '${kind}'|default): return \`([^\`]+)\``);
    const found = viewsSource.match(re);
    assert.ok(found, `${kind} 既没有专属 case 也没有兜底文案`);
    if (kind !== 'completed') {
      assert.ok(viewsSource.includes(`case '${kind}'`),
        `${kind} 没有显式分支——它会掉进 default 被显示成完成类事件`);
    }
    summaries.add(found[1].replace(/\$\{ev\.agent_name\}/, ''));
  }
  assert.equal(summaries.size, contract.event_kinds.length,
    '事件类型的界面文案有重复——用户分辨不出是哪一类');
  // 每一类都必须有**可操作的**外发开关：新类型默认开启，界面上没有开关
  // 就等于多出一条用户关不掉的对外发送通道（review 会读到死数据）。
  for (const [kind, key] of Object.entries(contract.event_kind_switches)) {
    assert.ok(viewsSource.includes(`'${key}'`),
      `views.js 没有渲染 ${kind} 的外发开关 ${key}——用户关不掉这类通知`);
  }
  // 开关必须在通知页**真实渲染**出来，而不只是出现在源码里。
  // 通知页在设置页内，用整页作用域查找 data-remote-policy 输入。
  const renderedKeys = await page.evaluate(() =>
    Array.from(document.querySelectorAll('[data-remote-policy]')).map(el => el.getAttribute('data-remote-policy')));
  for (const [kind, key] of Object.entries(contract.event_kind_switches)) {
    assert.ok(renderedKeys.includes(key),
      `通知页没有渲染 ${kind} 的开关 ${key}（实际渲染：${renderedKeys.join(', ')}）`);
  }
  // 提醒类呈现面必须认全部「需人处理」类型。本轮首版只有卡片认新类型，
  // 收起态摘要与窄条提醒对 question 完全无感——用户收起了岛就再也看不到它。
  const alertKinds = contract.event_kinds.filter(k => k !== 'completed');
  for (const kind of alertKinds) {
    assert.ok(viewsSource.includes(`isAlertEvent`),
      'views.js 缺少集中的提醒类型判定');
  }
  const alertSource = viewsSource.match(/const ALERT_EVENT_TYPES = \[([^\]]+)\]/);
  assert.ok(alertSource, 'views.js 应有 ALERT_EVENT_TYPES 清单');
  for (const kind of alertKinds) {
    assert.ok(alertSource[1].includes(`'${kind}'`),
      `ALERT_EVENT_TYPES 缺少 ${kind}——收起态/窄条不会提示它`);
  }
  // 两个呈现面都必须走同一判定，而不是各自硬编码
  const callSites = (viewsSource.match(/isAlertEvent\(/g) || []).length;
  assert.ok(callSites >= 3,
    `isAlertEvent 只被调用 ${callSites} 次——收起态与窄条应各自使用它`);
  // 交付三态（NOT-05：受理 ≠ 收到 ≠ 已读）：界面必须有四段文案，
  // 且「已受理」不得被说成「已送达」。**清单从契约夹具共读**——
  // 硬编码这份列表的话，夹具与界面之间就没有机制了（复审 A1 抓到过）：
  // 改名会同步夹具、绿掉两侧测试，而界面静默降级成「未确认」。
  assert.ok(Array.isArray(contract.delivery_states) && contract.delivery_states.length === 4,
    '契约夹具应给出四个交付状态');
  // 文案**也从夹具读**（NOT-05 复审 N2）：两侧共读同一份，改文案必须同时改夹具，
  // 否则 Rust 的 `label()` 与这里都会红。此前两侧各自硬编码字面量，
  // 可以「一致地」改成新文案而真机显示的是没人验证过的那份。
  assert.equal(typeof contract.delivery_labels?.accepted, 'string',
    '契约夹具应给出 delivery_labels.accepted（四态文案的唯一来源）');
  assert.equal(typeof contract.delivery_labels?.unknown, 'string',
    '契约夹具应给出 delivery_labels.unknown');
  // `Recent` 的对外字段名也是契约（复审 N3）：只改 Rust 的 serde 名会让界面静默显示空白。
  assert.ok(Array.isArray(contract.recent_fields) && contract.recent_fields.includes('deliveryLabel'),
    '契约夹具应把 Recent 的 deliveryLabel 列为契约字段');
  // 文案**只有 Rust 一处**（`DeliveryState::label`，经 `deliveryLabel` 下发）。
  // 界面若自己再维护一份映射就成了两份口径——复审 A4 抓到过：
  // 改了 Rust 侧、JS 侧不动，受理成功的消息会静默显示成「未确认」。
  assert.ok(viewsSource.includes('entry.deliveryLabel'),
    'views.js 必须使用 Rust 下发的 deliveryLabel，不得自建映射');
  for (const state of contract.delivery_states) {
    assert.ok(!new RegExp(`${state}:\\s*'`).test(viewsSource),
      `views.js 又自建了 ${state} 的文案映射——文案只能有一处（Rust 侧 label()）`);
  }
  // 真实渲染断言：注入一条受理记录与一条升级通道，检查**DOM**里真的出现
  // 交付状态文案与故障告警。此前夹具恒返回空数组，「字段被渲染」只在源码层面成立
  // （复审 D：读了不用、塞进隐藏元素都测不出）。
  // 夹具写进 sessionStorage（reload 会清空 window，但保留 sessionStorage）。
  const acceptedFixture = [{
    atMs: 1760000000000, title: 'Claude · 任务完成', text: '平台已受理',
    accepted: true, delivery: 'accepted', deliveryLabel: contract.delivery_labels.accepted,
  }];
  await page.evaluate(f => window.sessionStorage.setItem('recentFixture', JSON.stringify(f)), acceptedFixture);
  await page.reload();
  await page.waitForSelector('[data-remote-root][data-page-ready=true]');
  // 等异步渲染落地再断言（hydrateRemoteHistory 是 await invoke 的异步函数）。
  // 用 textContent 而非 innerText：设置页分区可能不可见，而 innerText 只返回**可见**文本，
  // 实测「渲染成功但 body.innerText 读不到」正是这个原因（调试时抓到过）。
  const historyBox = page.locator('[data-remote-history]');
  await historyBox.waitFor({ state: 'attached', timeout: 5000 });
  await page.waitForFunction(
    expected => {
      const box = document.querySelector('[data-remote-history]');
      return !!box && box.textContent.includes(expected);
    },
    contract.delivery_labels.accepted,
    { timeout: 5000 },
  ).catch(() => { throw new Error('受理记录的交付状态必须真的出现在 DOM 里（此前只在源码层面断言）'); });
  // 升级告警：注入一条升级通道，DOM 必须出现故障条
  await page.evaluate(f => {
    window.sessionStorage.setItem('recentFixture', JSON.stringify(f));
    window.sessionStorage.setItem('escalationsFixture', JSON.stringify(['ntfy']));
  }, acceptedFixture);
  await page.reload();
  await page.waitForSelector('[data-remote-root][data-page-ready=true]');
  // 故障条现在渲染在**高级设置之外**的独立容器里（N1 的修法），
  // 所以等的是那个容器，不是历史区。
  await page.waitForFunction(() => {
    const bar = document.querySelector('[data-remote-escalated]');
    return !!bar && bar.textContent.includes('停止自动重试');
  }, null, { timeout: 5000 })
    .catch(() => { throw new Error('通道升级后界面必须出现故障告警条'); });
  const escalatedText = await page.locator('[data-remote-escalated]').first().textContent();
  assert.ok(escalatedText.includes('ntfy'), '告警条要指名是哪个通道');
  // **可见性**（NOT-05 复审 N1）：故障条原先渲染在默认收起的
  // `<details data-remote-advanced>` 里——「现在就不工作」的信号被折叠起来，
  // 用户不展开高级设置就看不到。只测 textContent 是测不出这一点的
  // （实测：整块设成 display:none 时，textContent 断言照样通过）。
  const visibility = await page.evaluate(() => {
    const bar = document.querySelector('[data-remote-escalated]');
    if (!bar) return { found: false };
    const closedDetails = bar.closest('details:not([open])');
    return {
      found: true,
      insideClosedDetails: !!closedDetails,
      closedAncestor: closedDetails?.getAttribute('data-remote-advanced') !== null,
      checkVisibility: bar.checkVisibility
        ? bar.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true }) : null,
    };
  });
  assert.ok(visibility.found, '故障条必须存在');
  assert.ok(!visibility.insideClosedDetails,
    `故障条不得渲染在默认收起的 <details> 里（现在被折叠在 ${visibility.closedAncestor ? 'data-remote-advanced' : '某个 details'} 内），否则用户看不到`);
  assert.notEqual(visibility.checkVisibility, false, '故障条必须在布局中可见（checkVisibility 不得为 false）');
  assert.ok(escalatedText.includes('停止自动重试'), '告警条要说明已停止自动重试');
  // 清空夹具，恢复后续用例的页面状态
  await page.evaluate(() => {
    window.sessionStorage.removeItem('recentFixture');
    window.sessionStorage.removeItem('escalationsFixture');
  });
  await page.reload();
  await page.waitForSelector('[data-remote-root][data-page-ready=true]');
  // **故障清除后告警条必须消失**——上面只断言了「出现」。
  // 少了这条，把 else 分支写错（例如只清文案不 hidden，或干脆不清）
  // 会让一条陈旧的「通道故障」永久挂在页面上，而没有任何测试会响。
  // 陈旧故障比不显示更坏：用户会去排查一个已经好了的通道。
  const cleared = await page.evaluate(() => {
    const slot = document.querySelector('[data-remote-escalation-slot]');
    const bar = document.querySelector('[data-remote-escalated]');
    if (!slot) return { slotFound: false };
    const hidden = slot.hidden
      || getComputedStyle(slot).display === 'none'
      || (bar ? bar.checkVisibility
        ? !bar.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true }) : false : false);
    return { slotFound: true, hidden, barText: bar?.textContent ?? '' };
  });
  assert.ok(cleared.slotFound, '故障容器必须存在（清空后只是隐藏，不是移除）');
  assert.ok(cleared.hidden,
    `无故障时告警条必须隐藏/不可见，实际 barText="${cleared.barText}"——陈旧故障会让人去排查已经好了的通道`);
  assert.match(await group.innerText(), /样本不足|尚无样本|未判定/);
  const refresh = group.getByRole('button', { name: '刷新入队统计' });
  const ledger = group.locator('[data-enqueue-ledger]');
  assert.match(await ledger.innerText(), /重复抑制 未知/);
  assert.match(await group.innerText(), /1024条/);
  assert.match(await group.innerText(), /最近64条未确认提醒的摘要，不保存私密正文/);
  assert.match(await group.innerText(), /HTTP 受理不等于已保存/);
  assert.match(await group.innerText(), /最近入队可能丢失，最近已确认提醒可能再次出现/);
  const refreshLedger = async (diagnostics, expectedText) => {
    const before = await page.evaluate(value => {
      window.diagnostics = value;
      return window.calls.length;
    }, diagnostics);
    await refresh.click();
    await page.waitForFunction(text => document.querySelector('[data-enqueue-ledger]')?.textContent === text && !document.querySelector('[data-enqueue-refresh]')?.disabled, expectedText);
    assert.equal(await ledger.innerText(), expectedText);
    assert.deepEqual(await page.evaluate(start => window.calls.slice(start), before), ['get_engine_state'], '一次刷新只读取一次引擎状态');
  };
  for (const [sample, expected] of [
    [{ samples: 19, p95Ms: 4, discarded: 0, verdictAvailable: false, meetsTarget: null }, /样本不足|未判定/],
    [{ samples: 20, p95Ms: 1000, discarded: 0, verdictAvailable: true, meetsTarget: true }, /达标/],
    [{ samples: 20, p95Ms: 1001, discarded: 0, verdictAvailable: true, meetsTarget: false }, /未达标/],
    [{ samples: 20, p95Ms: 4, discarded: 1, verdictAvailable: false, meetsTarget: null }, /测量不完整|未判定/],
  ]) {
    await page.evaluate(value => { window.latency = value; window.latencyDelayMs = 100; }, sample);
    await refresh.click();
    const verdict = sample.discarded ? '测量不完整，未判定' : !sample.verdictAvailable ? '样本不足20条，未判定' : sample.meetsTarget ? '达标' : '未达标';
    const expectedText = `p95 ${sample.p95Ms}ms · 有效样本 ${sample.samples} · 丢弃 ${sample.discarded} · ${verdict}`;
    await page.waitForFunction(text => document.querySelector('[data-enqueue-summary]')?.textContent === text && !document.querySelector('[data-enqueue-refresh]')?.disabled, expectedText);
    const text = await group.locator('[data-enqueue-summary]').innerText();
    assert.equal(text, expectedText);
    assert.match(text, expected);
    if (sample.meetsTarget === null) assert(!text.split('·').map(s => s.trim()).includes('达标'), '未知不得宣布达标');
  }
  const ledgerLine = (duplicates, conflicts, unacked, evicted, restored, persistence) =>
    `重复抑制 ${duplicates} · id 复用冲突 ${conflicts} · 未确认 ${unacked} · 淘汰 ${evicted} · 本次启动恢复 ${restored} · 持久化：${persistence}`;
  await refreshLedger({ notify_duplicates: 7, notify_conflicts: 0, notify_unacked: 2, notify_evicted: 0, notify_restored: 0 }, ledgerLine(7, 0, 2, 0, 0, '未知'));
  await refreshLedger({ notify_duplicates: 0, notify_conflicts: 2, notify_unacked: 0, notify_evicted: 5, notify_restored: 3, notify_persistence: { enabled: true, pending: true } }, ledgerLine(0, 2, 0, 5, 3, '待保存（后台保存中）'));
  // Unrecognized shapes must not be presented as verified: a camelCase duplicate field
  // and `pending_events` (which excludes the visible head) are both "unknown", never 0.
  await refreshLedger({ notifyDuplicates: 9, pending_events: 3 }, ledgerLine('未知', '未知', '未知', '未知', '未知', '未知'));
  await refreshLedger({ notify_duplicates: 1, notify_conflicts: 0, notify_unacked: 1, notify_evicted: 0, notify_restored: 0, notify_persistence: { enabled: true, healthy: true, saved: true } }, ledgerLine(1, 0, 1, 0, 0, '已保存'));
  for (const [persistence, expected] of [
    [{ enabled: false }, '未启用'],
    [{ enabled: true, blocked: true, error: 'notify ledger unreadable; persistence disabled' }, '读取失败或账本被外部改动，已停写'],
    [{ enabled: true, error: 'notify ledger could not be saved' }, '保存失败'],
    [{ enabled: true, pending: true }, '待保存（后台保存中）'],
    [{ enabled: true, healthy: true, saved: true }, '已保存'],
    [{ enabled: true, healthy: true }, '尚无保存记录'],
  ]) {
    await refreshLedger({ notify_duplicates: 0, notify_persistence: persistence }, ledgerLine(0, '未知', '未知', '未知', '未知', expected));
  }
  // 未知字段名（camelCase 变体或 `notify_ledger` 摘要）不得被当成已核实口径展示。
  await refreshLedger({ notify_duplicates: 4, notify_persistence: { enabled: true, pending: true }, notify_ledger: { duplicates: 99, pending: 88, evicted: 77, restored: 66 } }, ledgerLine(4, '未知', '未知', '未知', '未知', '待保存（后台保存中）'));
  await page.evaluate(() => { window.latencyReadFails = true; });
  await refresh.click();
  await page.waitForFunction(() => /读不到|读取失败|暂不可用/.test(document.body.innerText));
  assert.match(await group.innerText(), /读不到|读取失败|暂不可用/);
  assert.equal(await ledger.innerText(), ledgerLine('未知', '未知', '未知', '未知', '未知', '未知'), '读取失败不得沿用旧保存成功状态或假装队列为空');
  await page.evaluate(() => { window.latencyReadFails = false; window.delayLatency = true; });
  await refresh.click();
  await page.waitForFunction(() => typeof window.finishLatency === 'function');
  await page.locator('[data-wb-nav="settings"]').first().click();
  await page.waitForFunction(() => !document.querySelector('[data-enqueue-summary]'));
  const retainedText = await page.evaluate(() => {
    window.finishLatency();
    return document.body.innerText;
  });
  await page.evaluate(() => new Promise(r => requestAnimationFrame(r)));
  assert(!await page.locator('[data-enqueue-summary]').count(), '迟到读取不得在另一页复活诊断');
  assert(retainedText.includes('设置'));
  await page.evaluate(() => { window.delayLatency = false; });
  await page.locator('[data-wb-nav="remote"]').first().click();
  await page.waitForSelector('[data-remote-root][data-page-ready=true]');
  await refreshLedger({ notify_duplicates: 22 }, ledgerLine(22, '未知', '未知', '未知', '未知', '未知'));
  const calls = await page.evaluate(() => window.calls);
  assert(!calls.some(c => /send_test|secret_set|patch_settings/.test(c)), '查看与刷新统计不得发送通知或改设置');
  assert.deepEqual(errors, []);
  console.log('PASS: notification latency UI — unknown/19 samples/boundary/slow/incomplete/read failure; refresh is read-only');
} finally {
  if (browser) await browser.close();
  await new Promise(r => server.close(r));
}
