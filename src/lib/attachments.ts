import { invoke } from "@tauri-apps/api/core";
import type { Attachment } from "../types";

/**
 * ファイル選択ダイアログを開き、選ばれた PDF を entry に添付する。
 * DetailPanel（サイドパネル）と DetailView（フルスクリーンリーダー）の
 * 双方から使う共通ロジック。ユーザーがダイアログをキャンセルしたら
 * `null` を返す（呼び出し側は成功扱いで無視できる）。
 */
export async function pickAndAttachPdf(entryId: number): Promise<Attachment | null> {
  const path = await invoke<string | null>("pick_pdf_file");
  if (!path) return null;
  return invoke<Attachment>("add_attachment", { entryId, sourcePath: path });
}
