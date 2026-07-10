// Service worker: ツールバーボタンのクリックで現在のタブから抽出 → /clipper へ POST。
// 結果はバッジ（+ ホバー時のタイトル文言）で通知する。

import { extractPage } from "./extract.js";
import { clip, loadConfig } from "./api.js";
import type { ClipPayload } from "./types.js";

const BADGE_CLEAR_MS = 4000;
let busy = false;
// バッジ世代番号（CR-029）。古い clear タイマーが新しいバッジを消さないようにする。
let badgeGen = 0;

function msg(key: string): string {
  return chrome.i18n.getMessage(key) || key;
}

async function showBadge(text: string, color: string, title: string): Promise<void> {
  const gen = ++badgeGen;
  await chrome.action.setBadgeBackgroundColor({ color });
  await chrome.action.setBadgeText({ text });
  await chrome.action.setTitle({ title });
  setTimeout(() => {
    // 自分より新しいバッジが出ていたら消さない。
    if (gen !== badgeGen) return;
    void chrome.action.setBadgeText({ text: "" });
    void chrome.action.setTitle({ title: msg("clipTitle") });
  }, BADGE_CLEAR_MS);
}

chrome.action.onClicked.addListener((tab) => {
  void handleClick(tab);
});

async function handleClick(tab: chrome.tabs.Tab): Promise<void> {
  if (busy || tab.id == null) return;
  busy = true;
  try {
    const cfg = await loadConfig();
    if (!cfg) {
      await showBadge("!", "#d93025", msg("errNotPaired"));
      void chrome.runtime.openOptionsPage();
      return;
    }

    // ページ側で抽出関数を実行（extractPage は自己完結な関数なので注入できる）
    let payload: ClipPayload;
    try {
      const [result] = await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        func: extractPage,
      });
      payload = result.result as ClipPayload;
    } catch {
      // chrome:// 等の注入不可ページ
      await showBadge("!", "#d93025", msg("errNoAccess"));
      return;
    }

    const outcome = await clip(cfg, payload);
    if (outcome.kind === "unreachable") {
      await showBadge("!", "#d93025", msg("errUnreachable"));
      return;
    }

    const { status, body } = outcome;
    if (status === 200 && body.status === "created") {
      await showBadge("✓", "#188038", `${msg("okCreated")}: ${body.title ?? ""}`);
    } else if (status === 200 && body.status === "duplicate") {
      await showBadge("=", "#f9ab00", `${msg("okDuplicate")}: ${body.title ?? ""}`);
    } else if (status === 401) {
      await showBadge("!", "#d93025", msg("errAuth"));
      void chrome.runtime.openOptionsPage();
    } else if (status === 403) {
      await showBadge("!", "#d93025", msg("errDisabled"));
    } else {
      await showBadge("!", "#d93025", `${msg("errServer")}: ${body.message ?? status}`);
    }
  } finally {
    busy = false;
  }
}
