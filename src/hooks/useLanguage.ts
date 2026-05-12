import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
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

export function useLanguage() {
  const { i18n } = useTranslation();
  const [setting, setSetting] = useState<LanguageSetting>(() => readStored());

  useEffect(() => {
    const effective = setting === "auto" ? resolveAuto() : setting;
    if (i18n.language !== effective) {
      void i18n.changeLanguage(effective);
    }
  }, [setting, i18n]);

  const setLanguage = useCallback((value: LanguageSetting) => {
    setSetting(value);
    try {
      localStorage.setItem(STORAGE_KEY, value);
    } catch {
      /* noop */
    }
  }, []);

  return {
    setting,
    effective: i18n.language as AppLanguage,
    setLanguage,
  };
}
