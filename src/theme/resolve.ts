import type { Appearance, ResolvedTheme, ThemeChoice } from "../lib/types";

export const THEMES: { id: ThemeChoice; name: string }[] = [
  { id: "system", name: "跟随系统" },
  { id: "light", name: "浅色" },
  { id: "dark", name: "深色" },
  { id: "paper", name: "纸感" },
  { id: "glass", name: "毛玻璃" },
  { id: "forest", name: "墨绿" },
];

/** 把用户选择解析为实际生效的主题：跟随系统取浅/深，毛玻璃不支持时降级为浅色 */
export function resolveTheme(
  choice: ThemeChoice,
  systemDark: boolean,
  glassSupported: boolean,
): ResolvedTheme {
  if (choice === "system") return systemDark ? "dark" : "light";
  if (choice === "glass" && !glassSupported) return "light";
  return choice;
}

/** 只需要 dataset 与 style 的最小接口，便于测试 */
export interface ThemeTarget {
  dataset: Record<string, string | undefined>;
  style: { setProperty(name: string, value: string): void; removeProperty(name: string): void };
}

export function applyAppearance(
  target: ThemeTarget,
  appearance: Appearance,
  systemDark: boolean,
  glassSupported: boolean,
): ResolvedTheme {
  const theme = resolveTheme(appearance.theme, systemDark, glassSupported);
  target.dataset.theme = theme;
  target.dataset.themeChoice = appearance.theme;
  target.dataset.fontsize = appearance.fontSize;
  target.dataset.radius = appearance.radius;
  target.dataset.motion = appearance.motion ? "on" : "off";
  if (appearance.accent) {
    target.style.setProperty("--accent", appearance.accent);
    const rgb = [1, 3, 5].map((i) => parseInt(appearance.accent!.slice(i, i + 2), 16) / 255);
    const linear = rgb.map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
    const luminance = linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
    target.style.setProperty("--on-accent", luminance > 0.179 ? "#000000" : "#ffffff");
  } else {
    target.style.removeProperty("--accent");
    target.style.removeProperty("--on-accent");
  }
  return theme;
}
