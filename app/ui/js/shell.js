// 形态判定：两个窗口各自从 URL 查询串上带着自己的形态
// （`tauri.conf.json` 里 island = `index.html?shell=island`、sidebar = `index.html?shell=sidebar`）。
//
// 为什么单独一个模块：`views.js` 与 `main.js` 都要读它，而 main.js 已经 import 了 views.js——
// 互相 import 会成环。形态是一种「进程级事实」，放这里最合适；**不是可变状态**：
// 一个窗口的形态在它被创建时就定了，切换形态是显示另一个窗口，不是改变这个窗口。
export const SHELL = (() => {
  try {
    const raw = new URLSearchParams(location.search).get('shell') ?? '';
    return raw.toLowerCase() === 'sidebar' ? 'sidebar' : 'island';
  } catch {
    // 取不到（例如非窗口环境里跑单测）就按灵动岛——那是默认形态
    return 'island';
  }
})();

export const isSidebar = () => SHELL === 'sidebar';
export const isIsland = () => SHELL === 'island';
