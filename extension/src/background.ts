// Service worker: ツールバーボタンのクリックで現在のタブから抽出 → /clipper へ POST。
// 結果はバッジ（+ ホバー時のタイトル文言）で通知する。
//
// 重複クリップでエントリに PDF / TeX ソースが欠けているとき、アプリ側の設定により
// (a) 自動補完（"completing"）か (b) 初回確認（"confirm_missing"）を返す。確認は
// ツールバーボタン直下のポップアップ（confirm.html）で取り、選択は confirm ページが
// メッセージで渡し、実際の /clipper/complete 呼び出し・状態遷移はすべてこの
// service worker が担う（ポップアップは閉じても補完が中断しないように・状態が壊れて
// ボタンが無反応にならないように、ネットワークと popup の arm/disarm を SW に集約する）。

import { extractPage } from "./extract.js";
import { clip, complete, loadConfig } from "./api.js";
import type { ClipPayload, PendingMissing } from "./types.js";

const BADGE_CLEAR_MS = 4000;
/** confirm ポップアップに渡す保留中の欠落補完（chrome.storage.session のキー）。 */
const PENDING_KEY = "pendingMissing";
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

/** ポップアップの arm を解除して保留を消す（ボタンを通常のクリップ動作へ戻す）。冪等。 */
async function disarmPopup(): Promise<void> {
  await chrome.storage.session.remove(PENDING_KEY);
  await chrome.action.setPopup({ popup: "" });
}

/** 欠落確認をツールバーボタン直下に出す。openPopup 不能時はバッジで誘導し次クリックで開く。 */
async function promptMissing(entryId: number, title: string, missing: string[]): Promise<void> {
  const pending: PendingMissing = { entry_id: entryId, title, missing };
  // 保留の書き込みと popup の arm を **openPopup より前に await 完了**させる
  // （でないと空の状態で開いて即閉じる／arm 前に開いて何も出ない）。
  await chrome.storage.session.set({ [PENDING_KEY]: pending });
  await chrome.action.setPopup({ popup: "confirm.html" });
  try {
    await chrome.action.openPopup();
  } catch {
    // Chrome <127 / フォーカス可能なウィンドウ無し等。popup は armed のままなので
    // 次のツールバークリックが confirm.html を開く（onClicked は popup 設定中は発火しない）。
    await showBadge("?", "#f9ab00", msg("confirmBadge"));
  }
}

/** confirm ページの選択を受けて実際に補完 API を叩く（popup が閉じても中断しない）。 */
async function runComplete(entryId: number, remember: boolean): Promise<void> {
  // まず状態をリセット（以降のクリックは通常どおりクリップに戻る）。
  await disarmPopup();
  const cfg = await loadConfig();
  if (!cfg) {
    await showBadge("!", "#d93025", msg("errNotPaired"));
    return;
  }
  const outcome = await complete(cfg, entryId, remember);
  if (outcome.kind === "unreachable") {
    await showBadge("!", "#d93025", msg("errUnreachable"));
  } else if (outcome.status === 200) {
    await showBadge("⇩", "#188038", msg("completing"));
  } else if (outcome.status === 401) {
    await showBadge("!", "#d93025", msg("errAuth"));
  } else if (outcome.status === 403) {
    await showBadge("!", "#d93025", msg("errDisabled"));
  } else {
    await showBadge("!", "#d93025", `${msg("errServer")}: ${outcome.body.message ?? outcome.status}`);
  }
}

// confirm ポップアップからのメッセージ。ネットワークと状態遷移は SW が持つ。
chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  const m = message as { type?: string; entryId?: number; remember?: boolean };
  if (m?.type === "clipperComplete" && typeof m.entryId === "number") {
    // true を返して応答待ちにすることで、fetch 完了まで SW が生かされる。
    void runComplete(m.entryId, !!m.remember).finally(() => sendResponse({ ok: true }));
    return true;
  }
  if (m?.type === "clipperCancel") {
    // 「今回はしない」/ フォーカス喪失での破棄 → arm を解除して通常動作へ戻す。
    void disarmPopup();
    return false;
  }
  return false;
});

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
      if (body.completing?.length) {
        // 設定 "1": アプリ側が欠落補完を自動で開始済み。
        await showBadge("⇩", "#188038", `${msg("completing")}: ${body.title ?? ""}`);
      } else if (body.confirm_missing?.length && body.entry_id != null) {
        // 初回確認: ボタン直下のポップアップで補完可否を尋ねる。
        await promptMissing(body.entry_id, body.title ?? "", body.confirm_missing);
      } else {
        await showBadge("=", "#f9ab00", `${msg("okDuplicate")}: ${body.title ?? ""}`);
      }
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
