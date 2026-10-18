/* 知了 · 原型共享脚本：主题偏好、图标、主题切换器 */
(function () {
  const STORE_KEY = "cicada-proto-prefs";

  const THEMES = [
    { id: "system", name: "跟随系统", sw: "linear-gradient(90deg,#F7F8F7 50%,#1E2223 50%)" },
    { id: "light", name: "浅色", sw: "#FFFFFF" },
    { id: "dark", name: "深色", sw: "#1E2223" },
    { id: "paper", name: "纸感", sw: "#F1E8D8" },
    { id: "glass", name: "毛玻璃", sw: "radial-gradient(at 20% 25%,#BFE3D8,transparent 60%),radial-gradient(at 80% 80%,#F6D9B8,transparent 60%),#E9EEEC" },
    { id: "forest", name: "墨绿", sw: "#1E2A26" },
  ];

  const DEFAULTS = { theme: "light", accent: null, fontsize: "medium", radius: "medium", motion: "on" };

  function load() {
    try {
      return Object.assign({}, DEFAULTS, JSON.parse(localStorage.getItem(STORE_KEY) || "{}"));
    } catch (e) {
      return Object.assign({}, DEFAULTS);
    }
  }

  const prefs = load();
  const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

  function resolvedTheme() {
    if (prefs.theme === "system") return darkQuery.matches ? "dark" : "light";
    return prefs.theme;
  }

  function apply() {
    const root = document.documentElement;
    root.dataset.theme = resolvedTheme();
    root.dataset.themeChoice = prefs.theme;
    root.dataset.fontsize = prefs.fontsize;
    root.dataset.radius = prefs.radius;
    root.dataset.motion = prefs.motion;
    if (prefs.accent) root.style.setProperty("--accent", prefs.accent);
    else root.style.removeProperty("--accent");
    document.dispatchEvent(new CustomEvent("cicada:prefs", { detail: prefs }));
  }

  function setPref(key, value) {
    prefs[key] = value;
    try { localStorage.setItem(STORE_KEY, JSON.stringify(prefs)); } catch (e) { /* 原型：存储不可用时忽略 */ }
    apply();
  }

  darkQuery.addEventListener("change", () => { if (prefs.theme === "system") apply(); });
  window.addEventListener("storage", (e) => {
    if (e.key !== STORE_KEY) return;
    Object.assign(prefs, load());
    apply();
  });

  apply();

  /* 线性图标（24×24，描边） */
  const ICONS = {
    settings: '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
    sticky: '<path d="M15.5 3H5a2 2 0 0 0-2 2v14c0 1.1.9 2 2 2h14a2 2 0 0 0 2-2V8.5L15.5 3Z"/><path d="M15 3v4a2 2 0 0 0 2 2h4"/>',
    minus: '<path d="M5 12h14"/>',
    square: '<rect x="5" y="5" width="14" height="14" rx="1.5"/>',
    x: '<path d="M18 6 6 18M6 6l12 12"/>',
    image: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.09-3.09a2 2 0 0 0-2.82 0L6 21"/>',
    "arrow-up": '<path d="m5 12 7-7 7 7M12 19V5"/>',
    undo: '<path d="M3 7v6h6"/><path d="M21 17a9 9 0 0 0-9-9 9 9 0 0 0-6 2.3L3 13"/>',
    bell: '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>',
    repeat: '<path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/>',
    link: '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>',
    "chevron-right": '<path d="m9 18 6-6-6-6"/>',
    "chevron-down": '<path d="m6 9 6 6 6-6"/>',
    folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
    tree: '<rect x="2" y="9" width="6" height="6" rx="1.5"/><rect x="16" y="3" width="6" height="6" rx="1.5"/><rect x="16" y="15" width="6" height="6" rx="1.5"/><path d="M8 12h4M12 6v12M12 6h4M12 18h4"/>',
    clock: '<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>',
    calendar: '<rect x="3" y="4" width="18" height="18" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/>',
    check: '<path d="M20 6 9 17l-5-5"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2m-7.07-17.07 1.41 1.41m11.32 11.32 1.41 1.41M2 12h2m16 0h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/>',
    pin: '<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>',
    lock: '<rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
    sparkles: '<path d="M12 3 13.9 8.1 19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9z"/><path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z"/>',
    inbox: '<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
    file: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4M16 13H8M16 17H8M10 9H8"/>',
    keyboard: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M6 8h.01M10 8h.01M14 8h.01M18 8h.01M8 12h.01M12 12h.01M16 12h.01M7 16h10"/>',
    power: '<path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.77.04"/>',
    database: '<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5"/><path d="M3 12a9 3 0 0 0 18 0"/>',
    info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/>',
    archive: '<rect x="2" y="3" width="20" height="5" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8M10 12h4"/>',
    plus: '<path d="M5 12h14M12 5v14"/>',
    droplet: '<path d="M12 22a7 7 0 0 0 7-7c0-2-1-3.9-3-5.5s-3.5-4-4-6.5c-.5 2.5-2 4.9-4 6.5C6 11.1 5 13 5 15a7 7 0 0 0 7 7z"/>',
    window: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M2 9h20"/>',
    alert: '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/><path d="M12 9v4M12 17h.01"/>',
    trash: '<path d="M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>',
    "cloud-off": '<path d="m2 2 20 20"/><path d="M5.78 5.78A7 7 0 0 0 9 19h8.5a4.5 4.5 0 0 0 1.31-.2"/><path d="M21.53 16.5A4.5 4.5 0 0 0 17.5 10h-1.79A7 7 0 0 0 9.22 5.28"/>',
    download: '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5M12 15V3"/>',
    upload: '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m17 8-5-5-5 5M12 3v12"/>',
    history: '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5M12 7v5l4 2"/>',
    refresh: '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
    zap: '<path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z"/>',
    moon: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
    message: '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>',
    more: '<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>',
    flag: '<path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1zM4 22v-7"/>',
    eye: '<path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/>',
  };

  function icon(name, cls) {
    const body = ICONS[name] || "";
    return '<svg class="icon ' + (cls || "") + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + body + "</svg>";
  }

  function mountIcons(root) {
    (root || document).querySelectorAll("[data-icon]").forEach((el) => {
      if (el.dataset.iconMounted) return;
      el.insertAdjacentHTML("afterbegin", icon(el.dataset.icon));
      el.dataset.iconMounted = "1";
    });
  }

  const CHECK_SVG = '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2.5 6.2 5 8.6 9.6 3.6"/></svg>';

  /* 原型专用：主题切换下拉菜单 */
  function mountThemeSwitcher(host) {
    host.classList.add("theme-switcher");
    host.innerHTML =
      '<button class="icon-btn" title="切换主题（原型）" aria-label="切换主题">' + icon("sun") + "</button>" +
      '<div class="theme-menu" role="menu"><div class="label">主题 · 原型切换器</div>' +
      THEMES.map((t) => '<button data-theme-id="' + t.id + '"><span class="sw" style="background:' + t.sw + '"></span>' + t.name + '<span class="tick">' + icon("check") + "</span></button>").join("") +
      "</div>";
    const btn = host.querySelector(".icon-btn");
    const menu = host.querySelector(".theme-menu");
    const sync = () => menu.querySelectorAll("[data-theme-id]").forEach((b) => b.classList.toggle("on", b.dataset.themeId === prefs.theme));
    btn.addEventListener("click", (e) => { e.stopPropagation(); menu.classList.toggle("open"); sync(); });
    menu.addEventListener("click", (e) => {
      const b = e.target.closest("[data-theme-id]");
      if (!b) return;
      setPref("theme", b.dataset.themeId);
      sync();
    });
    document.addEventListener("click", (e) => { if (!host.contains(e.target)) menu.classList.remove("open"); });
  }

  window.Cicada = { THEMES, prefs, setPref, icon, mountIcons, mountThemeSwitcher, CHECK_SVG, resolvedTheme };

  document.addEventListener("DOMContentLoaded", () => {
    mountIcons();
    document.querySelectorAll("[data-theme-switcher]").forEach(mountThemeSwitcher);
  });
})();
