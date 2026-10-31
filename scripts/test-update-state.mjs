import assert from 'node:assert/strict';
import { createUpdateController, updateStatusText, formatBytes } from '../app/ui/js/update-state.js';

// 本文件验证 NOT-06 的三条硬规则，全部是**可被突变推翻**的断言：
//   1. 只有用户点了「检查更新」才出网；
//   2. 只有处于 available 才允许安装，没有自动安装路径；
//   3. 安装完成**不**重启——必须再点一次。
// 第 3 条是本轮最容易写错的地方：如果 install() 里顺手调了 restart()，
// 用户会在毫不知情时被重启，而这类 bug 靠肉眼看界面是发现不了的。

const makeUpdate = (over = {}) => ({
  currentVersion: '0.0.326', available: true, version: '0.0.327', notes: '', date: '', ...over,
});

// --- 规则 1：只有显式点击才会检查 ------------------------------------------------
{
  let checks = 0;
  const controller = createUpdateController({ check: async () => { checks += 1; return makeUpdate(); } });
  assert.equal(checks, 0, '构造控制器不得触发检查');
  assert.equal(controller.state.phase, 'idle');
  await controller.check();
  assert.equal(checks, 1, '只有 check() 才检查');
  assert.equal(controller.state.phase, 'available');
}

// 没有可用的通道时，调用必须是无害的空操作，而不是抛错。
{
  const controller = createUpdateController({});
  await controller.check();
  await controller.install();
  await controller.restart();
  assert.equal(controller.state.phase, 'idle');
}

// --- 规则 2：只有 available 才允许安装 ------------------------------------------
{
  let installs = 0;
  const controller = createUpdateController({
    check: async () => makeUpdate(),
    install: async () => { installs += 1; },
  });
  // idle：没有检查过就不能安装
  await controller.install();
  assert.equal(installs, 0, '未检查时不得安装');
  // uptodate：没有新版本也不能安装
  await controller.check();
  controller.dismiss();
  const uptodate = createUpdateController({
    check: async () => makeUpdate({ available: false, version: null }),
    install: async () => { installs += 1; },
  });
  await uptodate.check();
  assert.equal(uptodate.state.phase, 'uptodate');
  await uptodate.install();
  assert.equal(installs, 0, '已是最新时不得安装');
  // ready（已装待重启）：不允许再装一次
  await controller.check();
  await controller.install();
  assert.equal(installs, 1);
  await controller.install();
  assert.equal(installs, 1, 'ready 状态不得重复安装');
}

// --- 规则 3：安装完成不重启 -----------------------------------------------------
{
  let restarts = 0;
  const controller = createUpdateController({
    check: async () => makeUpdate(),
    install: async () => {},
    restart: async () => { restarts += 1; },
  });
  await controller.check();
  await controller.install();
  assert.equal(controller.state.phase, 'ready');
  assert.equal(restarts, 0, '安装完成绝不能自动重启');
  assert.equal(controller.canRestart, true, '装完必须给用户一个显式重启入口');
  await controller.restart();
  assert.equal(restarts, 1);
}

// 重启前未安装成功时不得重启（否则会重启到一个没更新的版本，且丢掉现场）。
{
  let restarts = 0;
  const controller = createUpdateController({
    check: async () => makeUpdate(), install: async () => {}, restart: async () => { restarts += 1; },
  });
  await controller.check();
  await controller.restart();
  assert.equal(restarts, 0, '没安装就不该重启');
  await controller.install();
  await controller.restart();
  assert.equal(restarts, 1);
}

// 离开设置页后再进入，必须从后端恢复已安装待重启状态。
{
  let checks = 0, restarts = 0;
  const controller = createUpdateController({
    initialPhase: 'restoring',
    check: async () => { checks += 1; return makeUpdate(); },
    restart: async () => { restarts += 1; },
  });
  await controller.check();
  assert.equal(checks, 0, '恢复状态前不得发起检查');
  controller.restoreSessionStatus({ phase: 'ready' });
  assert.equal(controller.state.phase, 'ready');
  assert.equal(controller.canRestart, true);
  await controller.restart();
  assert.equal(restarts, 1, '恢复后的待重启状态仍允许用户显式重启');
}

{
  const controller = createUpdateController({ initialPhase: 'restoring' });
  controller.restoreSessionStatus({ phase: 'idle' });
  assert.equal(controller.state.phase, 'idle', '没有待重启更新时恢复为初始状态');
}

// 跳过由命令持久化；重进面板后可以恢复被跳过的提示，也能再次恢复更新通知。
{
  let skippedVersion = null;
  let skipCalls = 0, restoreCalls = 0;
  const controller = createUpdateController({
    check: async () => skippedVersion === '0.0.327'
      ? makeUpdate({ available: false, skipped: true, version: skippedVersion })
      : makeUpdate({ version: '0.0.327' }),
    skipVersion: async () => { skipCalls += 1; skippedVersion = '0.0.327'; return skippedVersion; },
    restoreSkippedVersion: async () => { restoreCalls += 1; skippedVersion = null; },
  });
  await controller.skipCurrentVersion();
  assert.equal(skipCalls, 0, '未检查前不能跳过一个客户端指定的版本');
  await controller.check();
  await controller.skipCurrentVersion();
  assert.equal(controller.state.phase, 'skipped');
  assert.equal(controller.state.skippedVersion, '0.0.327');
  assert.equal(controller.canInstall, false, '跳过之后不能继续安装该版本');

  await controller.check();
  assert.equal(controller.state.phase, 'skipped', '再次检查命中持久跳过版本时显示已跳过');
  assert.equal(controller.state.version, '0.0.327');
  await controller.restoreSkippedVersion();
  assert.equal(restoreCalls, 1);
  assert.equal(skippedVersion, null);
  assert.equal(controller.state.phase, 'idle', '恢复提示后要求用户再次显式检查');
  await controller.check();
  assert.equal(controller.state.phase, 'available', '恢复提示后再次检查可重新提供该版本');
}

{
  const controller = createUpdateController({ initialPhase: 'restoring' });
  controller.setSkippedVersion('0.0.327');
  controller.restoreSessionStatus({ phase: 'skipped' });
  assert.equal(controller.state.phase, 'skipped');
  assert.equal(controller.state.version, '0.0.327', '重进设置页恢复跳过版本号');
}

{
  const controller = createUpdateController({
    restoreSkippedVersion: async () => { throw new Error('设置文件只读'); },
  });
  controller.setSkippedVersion('0.0.327');
  await controller.restoreSkippedVersion();
  assert.equal(controller.state.skippedVersion, '0.0.327', '恢复失败时保留已保存的跳过选择');
  assert.match(updateStatusText(controller.state), /恢复更新提示失败：设置文件只读/);
}

{
  const controller = createUpdateController({
    initialPhase: 'restoring',
    restoreSkippedVersion: async () => {},
  });
  controller.setSkippedVersion('0.0.327');
  controller.restoreSessionStatus({ phase: 'failed', reason: '连接更新服务超时，请稍后重试' });
  await controller.restoreSkippedVersion();
  assert.equal(controller.state.phase, 'failed');
  assert.equal(controller.state.reason, '连接更新服务超时，请稍后重试', '恢复设置不能清掉检查失败原因');
  assert.match(updateStatusText(controller.state), /连接更新服务超时/);
}

{
  const controller = createUpdateController({ check: async () => makeUpdate({ version: '0.0.328' }) });
  controller.setNotice('读取跳过版本设置失败：设置不可用');
  await controller.check();
  assert.match(updateStatusText(controller.state), /读取跳过版本设置失败/, '检查结果不能替代成功重读设置的证据');
  controller.setSkippedVersion('0.0.327');
  assert.doesNotMatch(updateStatusText(controller.state), /读取跳过版本设置失败/, '仅成功读取设置后才清除错误');
}

{
  let resolveOldCheck;
  let checks = 0;
  const controller = createUpdateController({
    check: () => {
      checks += 1;
      if (checks === 1) return new Promise(resolve => { resolveOldCheck = resolve; });
      return Promise.resolve(makeUpdate({ version: '0.0.329' }));
    },
  });
  const oldCheck = controller.check();
  controller.beginSessionRestore();
  controller.restoreSessionStatus({ phase: 'idle' });
  await controller.check();
  assert.equal(controller.state.version, '0.0.329');
  resolveOldCheck(makeUpdate({ version: '0.0.328' }));
  await oldCheck;
  assert.equal(controller.state.version, '0.0.329', '重挂载后旧检查回包不得覆盖新检查');
}

{
  let rejectOldSkip;
  let checks = 0;
  const controller = createUpdateController({
    check: async () => makeUpdate({ version: checks++ === 0 ? '0.0.327' : '0.0.329' }),
    skipVersion: () => new Promise((resolve, reject) => { rejectOldSkip = reject; }),
  });
  await controller.check();
  const oldSkip = controller.skipCurrentVersion();
  controller.beginSessionRestore();
  controller.restoreSessionStatus({ phase: 'idle' });
  await controller.check();
  assert.equal(controller.state.phase, 'available');
  assert.equal(controller.state.version, '0.0.329');
  rejectOldSkip(new Error('旧面板的保存失败'));
  await oldSkip;
  assert.equal(controller.state.phase, 'available', '重挂载后旧跳过写入失败不得改写新阶段');
  assert.equal(controller.state.version, '0.0.329', '旧跳过写入失败不得覆盖新检查版本');
  assert.equal(controller.state.reason, '', '旧操作的错误不应泄漏到新面板');
}

{
  let resolveOldSkip;
  let checks = 0;
  const controller = createUpdateController({
    check: async () => makeUpdate({ version: checks++ === 0 ? '0.0.327' : '0.0.329' }),
    skipVersion: () => new Promise(resolve => { resolveOldSkip = resolve; }),
  });
  await controller.check();
  const oldSkip = controller.skipCurrentVersion();
  controller.beginSessionRestore();
  controller.setSkippedVersion('0.0.328');
  controller.restoreSessionStatus({ phase: 'idle' });
  await controller.check();
  resolveOldSkip('0.0.327');
  await oldSkip;
  assert.equal(controller.state.phase, 'available', '重挂载后旧跳过写入成功不得改写新阶段');
  assert.equal(controller.state.version, '0.0.329');
  assert.equal(controller.state.skippedVersion, '0.0.328', '旧跳过写入不得覆盖新读取的设置');
}

{
  let rejectOldRestore;
  const controller = createUpdateController({
    initialPhase: 'restoring',
    restoreSkippedVersion: () => new Promise((resolve, reject) => { rejectOldRestore = reject; }),
  });
  controller.setSkippedVersion('0.0.327');
  controller.restoreSessionStatus({ phase: 'skipped' });
  const oldRestore = controller.restoreSkippedVersion();
  controller.beginSessionRestore();
  controller.setSkippedVersion('0.0.328');
  controller.restoreSessionStatus({ phase: 'skipped' });
  rejectOldRestore(new Error('旧面板的恢复失败'));
  await oldRestore;
  assert.equal(controller.state.skippedVersion, '0.0.328', '重挂载后旧恢复回包不得清掉新读取的跳过版本');
  assert.equal(controller.state.phase, 'skipped');
  assert.equal(controller.state.notice, '', '旧恢复失败不能污染新面板');
}

{
  let resolveOldRestore;
  const controller = createUpdateController({
    initialPhase: 'restoring',
    restoreSkippedVersion: () => new Promise(resolve => { resolveOldRestore = resolve; }),
  });
  controller.setSkippedVersion('0.0.327');
  controller.restoreSessionStatus({ phase: 'skipped' });
  const oldRestore = controller.restoreSkippedVersion();
  controller.beginSessionRestore();
  controller.setSkippedVersion('0.0.328');
  controller.restoreSessionStatus({ phase: 'skipped' });
  resolveOldRestore();
  await oldRestore;
  assert.equal(controller.state.skippedVersion, '0.0.328', '重挂载后旧恢复成功不得清掉新读取的跳过版本');
  assert.equal(controller.state.phase, 'skipped');
}

{
  const controller = createUpdateController({ initialPhase: 'restoring' });
  controller.setSkippedVersion('0.0.327');
  controller.restoreSessionStatus({ phase: 'skipped' });
  controller.setNotice('读取跳过版本设置失败：设置不可用');
  assert.match(updateStatusText(controller.state), /已跳过版本 v0\.0\.327；读取跳过版本设置失败：设置不可用/);
}

{
  const controller = createUpdateController({ initialPhase: 'restoring' });
  controller.restoreSessionStatus({ phase: 'failed', reason: '网络不可用，无法连接更新服务' });
  assert.equal(controller.state.phase, 'failed');
  assert.equal(controller.state.reason, '网络不可用，无法连接更新服务', '重进设置页后保留脱敏失败原因');
}

{
  const controller = createUpdateController({ initialPhase: 'restoring' });
  controller.restoreSessionStatus({ phase: 'checking' });
  assert.equal(controller.state.phase, 'checking', '检查尚未完成时重进不能开放第二次检查');
  controller.restoreSessionStatus({ phase: 'idle' });
  assert.equal(controller.state.phase, 'idle', '检查完成后轮询应解除检查锁定');
}

// --- 失败与重试 ---------------------------------------------------------------
{
  let attempt = 0;
  const controller = createUpdateController({
    check: async () => {
      attempt += 1;
      if (attempt === 1) throw new Error('连接更新服务超时，请稍后重试');
      return makeUpdate();
    },
  });
  await controller.check();
  assert.equal(controller.state.phase, 'failed');
  assert.match(controller.state.reason, /超时/);
  await controller.check();
  assert.equal(controller.state.phase, 'available', '失败后可以重试');
}

// 安装失败不得停在「可重启」——否则用户会重启到一个半装的状态。
{
  let checks = 0;
  let dismissals = 0;
  const controller = createUpdateController({
    check: async () => { checks += 1; return makeUpdate(); },
    install: async () => { throw new Error('更新包签名校验失败，已拒绝安装'); },
    restart: async () => {},
    dismissRejection: async () => { dismissals += 1; },
  });
  await controller.check();
  await controller.install();
  assert.equal(controller.state.phase, 'rejected');
  assert.equal(controller.canRestart, false, '安装失败不得显示重启入口');
  assert.match(controller.state.reason, /签名/);
  await controller.check();
  assert.equal(checks, 1, '签名拒绝后不得直接重试');
  await controller.closeRejection();
  assert.equal(dismissals, 1, '清除后端拒绝状态必须来自用户关闭动作');
  await controller.check();
  assert.equal(checks, 2, '用户关闭拒绝提示后可重新手动检查');
}

// 畸形响应按失败处理，不得当成「有更新」。
for (const bad of [null, {}, { available: 'yes' }, { available: 1 },
  { available: false, skipped: 'yes' }, { available: false, skipped: true, version: null },
  { available: true, skipped: true, version: '0.0.327' }]) {
  const controller = createUpdateController({ check: async () => bad });
  await controller.check();
  assert.equal(controller.state.phase, 'failed', `畸形响应 ${JSON.stringify(bad)} 不得被当成有更新`);
}

// 并发点击：busy 期间不重复触发。
{
  let checks = 0, release;
  const gate = new Promise(resolve => { release = resolve; });
  const controller = createUpdateController({ check: async () => { checks += 1; await gate; return makeUpdate(); } });
  const first = controller.check();
  await controller.check();
  await controller.install();
  assert.equal(checks, 1, 'busy 期间不得重复检查');
  release();
  await first;
}

// --- 进度事件 ------------------------------------------------------------------
{
  const controller = createUpdateController({ check: async () => makeUpdate(), install: async () => {} });
  await controller.check();
  await controller.install();
  assert.equal(controller.state.phase, 'ready');
  // 装完之后到达的迟到进度不得把界面拽回下载中
  controller.progress(1024, 2048);
  assert.equal(controller.state.phase, 'ready', '迟到的进度事件不得回退已完成的安装');
}

{
  const controller = createUpdateController({ check: async () => makeUpdate(), install: async () => {} });
  await controller.check();
  const installing = controller.install();
  controller.progress(0, null);
  assert.equal(controller.state.downloaded, 0);
  assert.equal(controller.state.total, null, '总大小未知时不得编一个数字');
  controller.progress(2048, 4096);
  assert.equal(controller.state.total, 4096);
  controller.progress(NaN, -1);
  assert.equal(controller.state.downloaded, null, '非法进度不得进界面');
  await installing;
}

// --- 文案 ----------------------------------------------------------------------
assert.equal(updateStatusText({ phase: 'idle' }), '尚未检查更新');
assert.equal(updateStatusText({ phase: 'idle', skippedVersion: '0.0.327' }), '已跳过版本 v0.0.327');
assert.equal(updateStatusText({ phase: 'idle', skippedVersion: '0.0.327', notice: '恢复更新提示失败：设置文件只读' }), '已跳过版本 v0.0.327；恢复更新提示失败：设置文件只读');
assert.equal(updateStatusText({ phase: 'restoring' }), '正在读取更新状态…');
assert.equal(updateStatusText({ phase: 'checking' }), '正在检查更新…');
assert.equal(updateStatusText({ phase: 'uptodate' }), '已是最新版本');
assert.equal(updateStatusText({ phase: 'skipped', version: '0.0.327' }), '已跳过版本 v0.0.327');
assert.equal(updateStatusText({ phase: 'available', version: '0.0.327' }), '发现新版本 v0.0.327');
assert.equal(updateStatusText({ phase: 'available', version: '0.0.327', notice: '恢复更新提示失败：设置文件只读' }), '发现新版本 v0.0.327；恢复更新提示失败：设置文件只读');
assert.equal(updateStatusText({ phase: 'downloading', downloaded: null }), '正在下载更新包…');
assert.match(updateStatusText({ phase: 'downloading', downloaded: 1024, total: null }), /已下载 1\.0 KB/);
assert.match(updateStatusText({ phase: 'downloading', downloaded: 1024, total: 4096 }), /^正在下载 25%/);
assert.equal(updateStatusText({ phase: 'ready' }), '更新已安装，重启后生效');
assert.equal(updateStatusText({ phase: 'failed', reason: 'x' }), 'x');
assert.equal(updateStatusText({ phase: 'failed' }), '更新失败，请稍后重试');
assert.equal(updateStatusText({ phase: 'rejected' }), '更新包签名校验失败，已拒绝安装');
assert.equal(updateStatusText(null), '尚未检查更新');

assert.equal(formatBytes(0), '0 B');
assert.equal(formatBytes(512), '512 B');
assert.equal(formatBytes(2048), '2.0 KB');
assert.equal(formatBytes(5 * 1024 * 1024), '5.0 MB');
assert.equal(formatBytes(-1), '—');
assert.equal(formatBytes('x'), '—');

console.log('update-state: all assertions passed');
