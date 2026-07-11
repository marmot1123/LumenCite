import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";

import ja from "./locales/ja.json";
import en from "./locales/en.json";

export const resources = {
  ja: { translation: ja },
  en: { translation: en },
} as const;

export type AppLanguage = keyof typeof resources;

const STORAGE_KEY = "lc-language";

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources,
    fallbackLng: "ja",
    supportedLngs: ["ja", "en"],
    interpolation: { escapeValue: false },
    detection: {
      order: ["localStorage", "navigator"],
      lookupLocalStorage: STORAGE_KEY,
      caches: ["localStorage"],
    },
  });

// HTML の lang 属性を実際の言語に同期する（CR-038）。index.html は lang="en" 固定で、
// 日本語表示でも en のままだった（スクリーンリーダーの読み・ハイフネーションが不正確）。
function syncHtmlLang(lng: string) {
  if (typeof document !== "undefined") {
    document.documentElement.lang = lng.startsWith("ja") ? "ja" : "en";
  }
}
i18n.on("languageChanged", syncHtmlLang);
i18n.on("initialized", () => syncHtmlLang(i18n.language));

export default i18n;

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    resources: { translation: typeof ja };
  }
}
