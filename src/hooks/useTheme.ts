import { useState, useEffect } from "react";
import type { ThemeMode, AccentName, Density } from "../types";

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
};

const LIGHT_TOKENS: Record<string, string> = {
  "--bg": "oklch(0.985 0.003 80)", "--surface": "#ffffff",
  "--surface-2": "oklch(0.975 0.004 80)", "--sidebar": "oklch(0.972 0.004 80)",
  "--border": "oklch(0.92 0.005 80)", "--border-subtle": "oklch(0.95 0.004 80)",
  "--border-strong": "oklch(0.86 0.006 80)", "--text": "oklch(0.22 0.01 70)",
  "--text-mute": "oklch(0.5 0.008 70)", "--text-faint": "oklch(0.65 0.005 70)",
  "--row-hover": "oklch(0.965 0.005 80)", "--row-selected": "oklch(0.955 0.02 70)",
  "--hover": "oklch(0.95 0.005 80)",
};

function applyTheme(theme: ThemeMode, accent: AccentName) {
  const tokens = theme === "dark" ? DARK_TOKENS : LIGHT_TOKENS;
  const a = ACCENTS[accent];
  const v = document.documentElement.style;
  Object.entries(tokens).forEach(([k, val]) => v.setProperty(k, val));
  v.setProperty("--accent-strong", theme === "dark" ? "oklch(0.74 0.12 65)" : a.strong);
  v.setProperty("--accent-soft", theme === "dark" ? "oklch(0.36 0.05 65)" : a.soft);
  v.setProperty("--accent-ring", a.ring);
}

function ls<T>(key: string, fallback: T): T {
  try { const v = localStorage.getItem(key); return v ? (JSON.parse(v) as T) : fallback; }
  catch { return fallback; }
}

export function useTheme() {
  const [theme, setThemeState] = useState<ThemeMode>(() => ls("lc-theme", "light"));
  const [accent, setAccentState] = useState<AccentName>(() => ls("lc-accent", "amber"));
  const [density, setDensityState] = useState<Density>(() => ls("lc-density", "default"));

  useEffect(() => { applyTheme(theme, accent); }, [theme, accent]);

  const setTheme = (t: ThemeMode) => { setThemeState(t); localStorage.setItem("lc-theme", JSON.stringify(t)); };
  const setAccent = (a: AccentName) => { setAccentState(a); localStorage.setItem("lc-accent", JSON.stringify(a)); };
  const setDensity = (d: Density) => { setDensityState(d); localStorage.setItem("lc-density", JSON.stringify(d)); };

  return { theme, accent, density, setTheme, setAccent, setDensity };
}
