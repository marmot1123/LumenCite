// 欠落補完の確認ポップアップ（ツールバーボタン直下）。
//
// このページは「表示と選択の受け渡し」だけを行う純粋なビュー。実際の
// /clipper/complete 呼び出しと popup の arm/disarm は service worker が担う
// （ポップアップはフォーカス喪失で即座に破棄され得るため、ネットワークをここに
// 置くと補完が黙って中断する）。どの終了経路でも SW にメッセージを送り、SW が
// ボタン状態を通常へ戻す。

import type { PendingMissing } from "./types.js";

const PENDING_KEY = "pendingMissing";
/** 選択済みなら true。pagehide での二重キャンセル送信を防ぐ。 */
let handled = false;

function msg(key: string): string {
  return chrome.i18n.getMessage(key) || key;
}

function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing element #${id}`);
  return el;
}

function send(message: unknown): void {
  try {
    // 応答は待たない（このページは直後に閉じる）。SW 側が受けて処理する。
    void chrome.runtime.sendMessage(message);
  } catch {
    // SW 起動途中でも Chrome がメッセージをキューするため握り潰してよい。
  }
}

/** 選択を SW へ渡してポップアップを閉じる。 */
function finish(message: unknown): void {
  handled = true;
  send(message);
  window.close();
}

function labelFor(kind: string): string {
  if (kind === "pdf") return msg("confirmMissingPdf");
  if (kind === "tex") return msg("confirmMissingTex");
  return kind;
}

async function init(): Promise<void> {
  const stored = await chrome.storage.session.get(PENDING_KEY);
  const pending = stored[PENDING_KEY] as PendingMissing | undefined;
  if (!pending) {
    // 保留が無い（処理済み / 異常）→ SW にボタン復帰を依頼して閉じる（stuck 防止）。
    finish({ type: "clipperCancel" });
    return;
  }

  document.title = msg("confirmTitle");
  $("title").textContent = msg("confirmTitle");
  $("body").textContent = msg("confirmBody");
  $("entry").textContent = pending.title;
  $("missing").textContent = pending.missing.map(labelFor).join(" ・ ");

  const yes = $("complete");
  const skip = $("skip");
  const always = $("always");
  yes.textContent = msg("confirmComplete");
  skip.textContent = msg("confirmSkip");
  always.textContent = msg("confirmAlways");

  yes.addEventListener("click", () =>
    finish({ type: "clipperComplete", entryId: pending.entry_id, remember: false }),
  );
  always.addEventListener("click", () =>
    finish({ type: "clipperComplete", entryId: pending.entry_id, remember: true }),
  );
  skip.addEventListener("click", () => finish({ type: "clipperCancel" }));
}

// 選択せずに閉じられた（フォーカス喪失・Esc）場合も SW に通知して arm を解除する。
window.addEventListener("pagehide", () => {
  if (!handled) send({ type: "clipperCancel" });
});

void init();
