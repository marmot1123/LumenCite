import { useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import i18n from "../i18n";
import type { AppLanguage } from "../i18n";

const STORAGE_KEY = "lc-language";

type LanguageSetting = AppLanguage | "auto";

function resolveAuto(): AppLanguage {
  const nav = typeof navigator !== "undefined" ? navigator.language : "ja";
  return nav.toLowerCase().startsWith("ja") ? "ja" : "en";
}

function readStored(): LanguageSetting {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === "ja" || raw === "en" || raw === "auto") return raw;
  } catch {
    /* noop */
  }
  return "auto";
}

// 言語設定はモジュールレベルで共有する（useTheme と同じ理由：フックごとの
// useState だと、コマンドパレットで切り替えても設定モーダルの表示が古いまま残る）。
let setting: LanguageSetting = readStored();
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function apply(s: LanguageSetting) {
  const effective = s === "auto" ? resolveAuto() : s;
  if (i18n.language !== effective) {
    void i18n.changeLanguage(effective);
  }
}

// 起動時に保存済み設定を反映（従来は最初のフックのマウント時に行っていた）
apply(setting);

const setLanguage = (value: LanguageSetting) => {
  setting = value;
  try {
    localStorage.setItem(STORAGE_KEY, value);
  } catch {
    /* noop */
  }
  apply(value);
  listeners.forEach((l) => l());
};

export function useLanguage() {
  // useTranslation を挟むことで言語変更時に再レンダーされ、effective が追従する
  const { i18n: i18nHook } = useTranslation();
  const s = useSyncExternalStore(subscribe, () => setting);
  return {
    setting: s,
    effective: i18nHook.language as AppLanguage,
    setLanguage,
  };
}
