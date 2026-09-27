// 形态判定：两个窗口加载同一个 index.html，形态由 **`<html>` 上的类**决定
// （`shell-island` / `shell-sidebar`），那个类由 index.html 里的内联脚本在第一次绘制之前挂上。
//
// 为什么以类为准、而不是每次自己解析查询串：类必须**早早设好**（否则样式会闪一下），
// 那就该只有一个地方设它、其他地方读它。查询串作为回落，只在类缺失时用
// （例如有人在非窗口环境里直接加载这个模块）。
//
// 为什么单独一个模块：`views.js` 与 `main.js` 都要读它，而 main.js 已经 import 了 views.js——
// 互相 import 会成环。形态是一种「进程级事实」，**不是可变状态**：
// 一个窗口的形态在它被创建时就定了，切换形态是显示另一个窗口，不是改变这个窗口。
export const SHELL = (() => {
  try {
    if (document.documentElement.classList.contains('shell-sidebar')) return 'sidebar';
    if (document.documentElement.classList.contains('shell-island')) return 'island';
    const raw = new URLSearchParams(location.search).get('shell') ?? '';
    return raw.toLowerCase() === 'sidebar' ? 'sidebar' : 'island';
  } catch {
    // 取不到（例如非窗口环境里跑单测）就按灵动岛——那是默认形态
    return 'island';
  }
})();
export const isSidebar = () => SHELL === 'sidebar';
export const isIsland = () => SHELL === 'island';
