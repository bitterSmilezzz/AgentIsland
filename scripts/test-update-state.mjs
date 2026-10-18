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
  const controller = createUpdateController({
    check: async () => makeUpdate(),
    install: async () => { throw new Error('更新包签名校验失败，已拒绝安装'); },
    restart: async () => {},
  });
  await controller.check();
  await controller.install();
  assert.equal(controller.state.phase, 'failed');
  assert.equal(controller.canRestart, false, '安装失败不得显示重启入口');
  assert.match(controller.state.reason, /签名/);
}

// 畸形响应按失败处理，不得当成「有更新」。
for (const bad of [null, {}, { available: 'yes' }, { available: 1 }]) {
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
assert.equal(updateStatusText({ phase: 'checking' }), '正在检查更新…');
assert.equal(updateStatusText({ phase: 'uptodate' }), '已是最新版本');
assert.equal(updateStatusText({ phase: 'available', version: '0.0.327' }), '发现新版本 v0.0.327');
assert.equal(updateStatusText({ phase: 'downloading', downloaded: null }), '正在下载更新包…');
assert.match(updateStatusText({ phase: 'downloading', downloaded: 1024, total: null }), /已下载 1\.0 KB/);
assert.match(updateStatusText({ phase: 'downloading', downloaded: 1024, total: 4096 }), /^正在下载 25%/);
assert.equal(updateStatusText({ phase: 'ready' }), '更新已安装，重启后生效');
assert.equal(updateStatusText({ phase: 'failed', reason: 'x' }), 'x');
assert.equal(updateStatusText({ phase: 'failed' }), '更新失败，请稍后重试');
assert.equal(updateStatusText(null), '尚未检查更新');

assert.equal(formatBytes(0), '0 B');
assert.equal(formatBytes(512), '512 B');
assert.equal(formatBytes(2048), '2.0 KB');
assert.equal(formatBytes(5 * 1024 * 1024), '5.0 MB');
assert.equal(formatBytes(-1), '—');
assert.equal(formatBytes('x'), '—');

console.log('update-state: all assertions passed');
