import { useSyncExternalStore } from "react";
import type { ThemeMode, AccentName, Density, ResolvedTheme } from "../types";

const ACCENTS: Record<AccentName, { strong: string; soft: string; ring: string }> = {
  amber:  { strong: "oklch(0.62 0.14 65)",  soft: "oklch(0.95 0.04 70)",  ring: "oklch(0.7 0.13 65 / 0.25)" },
  indigo: { strong: "oklch(0.52 0.16 270)", soft: "oklch(0.95 0.04 270)", ring: "oklch(0.6 0.15 270 / 0.25)" },
  teal:   { strong: "oklch(0.55 0.10 195)", soft: "oklch(0.95 0.04 200)", ring: "oklch(0.62 0.10 200 / 0.25)" },
  rose:   { strong: "oklch(0.58 0.16 15)",  soft: "oklch(0.95 0.04 15)",  ring: "oklch(0.65 0.15 15 / 0.25)" },
};

const DARK_TOKENS: Record<string, string> = {
  "--bg": "oklch(0.27 0.004 80)", "--surface": "oklch(0.31 0.004 80)",
  "--surface-2": "oklch(0.29 0.004 80)", "--sidebar": "oklch(0.285 0.004 80)",
  "--border": "oklch(0.38 0.004 80)", "--border-subtle": "oklch(0.34 0.004 80)",
  "--border-strong": "oklch(0.44 0.004 80)", "--text": "oklch(0.86 0.004 80)",
  "--text-mute": "oklch(0.66 0.004 80)", "--text-faint": "oklch(0.52 0.004 80)",
  "--row-hover": "oklch(0.34 0.004 80)", "--row-selected": "oklch(0.38 0.018 70)",
  "--hover": "oklch(0.34 0.004 80)",
  // 状態色（赤＝危険／エラー、黄＝警告／重複、緑＝成功）
  "--danger-bg":    "oklch(0.30 0.04 15)",
  "--danger-border":"oklch(0.45 0.08 15)",
  "--danger-text":  "oklch(0.85 0.10 15)",
  "--danger-strong":"oklch(0.6 0.18 15)",
  "--warn-bg":      "oklch(0.30 0.04 70)",
  "--warn-border":  "oklch(0.50 0.10 70)",
  "--warn-text":    "oklch(0.85 0.12 70)",
  "--warn-strong":  "oklch(0.62 0.15 70)",
  "--success-text": "oklch(0.78 0.12 145)",
};

const LIGHT_TOKENS: Record<string, string> = {
  "--bg": "oklch(0.985 0.003 80)", "--surface": "#ffffff",
  "--surface-2": "oklch(0.975 0.004 80)", "--sidebar": "oklch(0.972 0.004 80)",
  "--border": "oklch(0.92 0.005 80)", "--border-subtle": "oklch(0.95 0.004 80)",
  "--border-strong": "oklch(0.86 0.006 80)", "--text": "oklch(0.22 0.01 70)",
  "--text-mute": "oklch(0.5 0.008 70)", "--text-faint": "oklch(0.65 0.005 70)",
  "--row-hover": "oklch(0.965 0.005 80)", "--row-selected": "oklch(0.955 0.02 70)",
  "--hover": "oklch(0.95 0.005 80)",
  "--danger-bg":    "oklch(0.96 0.03 15)",
  "--danger-border":"oklch(0.88 0.06 15)",
  "--danger-text":  "oklch(0.45 0.13 15)",
  "--danger-strong":"oklch(0.55 0.18 15)",
  "--warn-bg":      "oklch(0.96 0.06 70)",
  "--warn-border":  "oklch(0.85 0.13 70)",
  "--warn-text":    "oklch(0.35 0.10 70)",
  "--warn-strong":  "oklch(0.55 0.15 70)",
  "--success-text": "oklch(0.55 0.12 145)",
};

function applyTheme(resolved: ResolvedTheme, accent: AccentName) {
  const tokens = resolved === "dark" ? DARK_TOKENS : LIGHT_TOKENS;
  const a = ACCENTS[accent];
  const v = document.documentElement.style;
  Object.entries(tokens).forEach(([k, val]) => v.setProperty(k, val));
  v.setProperty("--accent-strong", resolved === "dark" ? "oklch(0.74 0.12 65)" : a.strong);
  v.setProperty("--accent-soft", resolved === "dark" ? "oklch(0.36 0.05 65)" : a.soft);
  v.setProperty("--accent-ring", a.ring);
  document.documentElement.dataset.theme = resolved;
}

function readSystemTheme(): ResolvedTheme {
  if (typeof window === "undefined" || !window.matchMedia) return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function ls<T>(key: string, fallback: T): T {
  try { const v = localStorage.getItem(key); return v ? (JSON.parse(v) as T) : fallback; }
  catch { return fallback; }
}

// テーマ状態はモジュールレベルの単一ストアで共有する。
// フックごとに独立 useState を持つと、設定モーダルでの変更が他のコンポーネント
// （App の density、サイドバー等）に伝播せず、App 側に残った古い "auto" が
// OS のライト/ダーク切替時にユーザーの明示的な選択を上書きしてしまう。
type ThemeState = {
  theme: ThemeMode;
  accent: AccentName;
  density: Density;
  systemTheme: ResolvedTheme;
};

let state: ThemeState = {
  theme: ls("lc-theme", "auto"),
  accent: ls("lc-accent", "amber"),
  density: ls("lc-density", "default"),
  systemTheme: readSystemTheme(),
};

const listeners = new Set<() => void>();

function resolvedOf(s: ThemeState): ResolvedTheme {
  return s.theme === "auto" ? s.systemTheme : s.theme;
}

function setState(patch: Partial<ThemeState>) {
  state = { ...state, ...patch };
  applyTheme(resolvedOf(state), state.accent);
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

if (typeof window !== "undefined" && window.matchMedia) {
  // OS のライト/ダーク切替に追従（モジュールで一度だけ購読）
  window
    .matchMedia("(prefers-color-scheme: dark)")
    .addEventListener("change", (e) => setState({ systemTheme: e.matches ? "dark" : "light" }));
  // 別ウィンドウ（PDF ビューワー）からの変更にも追従する
  window.addEventListener("storage", (e) => {
    if (e.key === "lc-theme") setState({ theme: ls("lc-theme", "auto") });
    if (e.key === "lc-accent") setState({ accent: ls("lc-accent", "amber") });
    if (e.key === "lc-density") setState({ density: ls("lc-density", "default") });
  });
  // 初期適用（従来はマウント時 effect で行っていた）
  applyTheme(resolvedOf(state), state.accent);
}

const setTheme = (t: ThemeMode) => {
  localStorage.setItem("lc-theme", JSON.stringify(t));
  setState({ theme: t });
};
const setAccent = (a: AccentName) => {
  localStorage.setItem("lc-accent", JSON.stringify(a));
  setState({ accent: a });
};
const setDensity = (d: Density) => {
  localStorage.setItem("lc-density", JSON.stringify(d));
  setState({ density: d });
};

export function useTheme() {
  const s = useSyncExternalStore(subscribe, () => state);
  return {
    theme: s.theme,
    accent: s.accent,
    density: s.density,
    resolved: resolvedOf(s),
    setTheme,
    setAccent,
    setDensity,
  };
}
