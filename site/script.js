// AgentIsland 站点：外观切换 + 滚动进场 + 导航当前位置
(function () {
  var root = document.documentElement;
  var KEY = "agentisland-site-theme";

  function preferred() {
    var forced = /[?&]theme=(light|dark)/.exec(location.search);
    if (forced) return forced[1];
    var saved = null;
    try { saved = localStorage.getItem(KEY); } catch (e) {}
    if (saved === "light" || saved === "dark") return saved;
    return window.matchMedia && window.matchMedia("(prefers-color-scheme: light)").matches
      ? "light" : "dark";
  }

  function apply(mode) {
    root.setAttribute("data-theme", mode);
    var meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute("content", mode === "light" ? "#f5f5f7" : "#0c0d14");
  }

  var mode = preferred();
  apply(mode);

  var toggle = document.getElementById("theme-toggle");
  if (toggle) {
    toggle.addEventListener("click", function () {
      mode = root.getAttribute("data-theme") === "light" ? "dark" : "light";
      apply(mode);
      try { localStorage.setItem(KEY, mode); } catch (e) {}
    });
  }

  var reveals = Array.prototype.slice.call(document.querySelectorAll(".reveal"));
  if ("IntersectionObserver" in window && reveals.length) {
    var seen = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (!entry.isIntersecting) return;
        entry.target.classList.add("in");
        seen.unobserve(entry.target);
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.08 });
    reveals.forEach(function (el) { seen.observe(el); });
  } else {
    reveals.forEach(function (el) { el.classList.add("in"); });
  }

  // 灵动岛演示：滚进视野先收起、再弹开一次；之后可点/可键盘聚焦反复开合
  var demo = document.getElementById("island-demo");
  if (demo) {
    var reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduce) {
      demo.classList.add("is-open");
    } else {
      var played = false;
      var open = function () { demo.classList.add("is-open"); };
      var play = function () {
        if (played) return;
        played = true;
        demo.classList.remove("is-open");
        setTimeout(open, 620);
      };
      if ("IntersectionObserver" in window) {
        var io = new IntersectionObserver(function (entries) {
          entries.forEach(function (e) { if (e.isIntersecting) { play(); io.disconnect(); } });
        }, { threshold: 0.45 });
        io.observe(demo);
      } else {
        setTimeout(open, 620);
      }
      demo.addEventListener("click", function () { demo.classList.toggle("is-open"); });
      demo.addEventListener("keydown", function (e) {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          demo.classList.toggle("is-open");
        }
      });
    }
  }

  var links = Array.prototype.slice.call(document.querySelectorAll(".nav a"));
  var targets = links
    .map(function (a) { return document.querySelector(a.getAttribute("href")); })
    .filter(Boolean);
  if ("IntersectionObserver" in window && targets.length) {
    var spy = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (!entry.isIntersecting) return;
        links.forEach(function (a) {
          var on = a.getAttribute("href") === "#" + entry.target.id;
          a.style.color = on ? "var(--ink)" : "";
          if (on) { a.setAttribute("aria-current", "true"); } else { a.removeAttribute("aria-current"); }
        });
      });
    }, { rootMargin: "-45% 0px -50% 0px" });
    targets.forEach(function (t) { spy.observe(t); });
  }
})();
