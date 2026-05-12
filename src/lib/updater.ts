import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export interface UpdateAvailable {
  status: "available";
  version: string;
  date: string | null;
  body: string | null;
  currentVersion: string;
  update: Update;
}

export interface UpdateNotAvailable {
  status: "up_to_date";
  currentVersion: string;
}

export interface UpdateError {
  status: "error";
  message: string;
}

export type UpdateCheckResult = UpdateAvailable | UpdateNotAvailable | UpdateError;

/**
 * 最新版があるかバックエンドに問い合わせる。
 * 結果は status で分岐できる union type で返す（throw しない）。
 */
export async function checkForUpdate(): Promise<UpdateCheckResult> {
  try {
    const update = await check();
    if (!update) {
      // Tauri plugin はバージョンを current として取れないので、placeholder
      return { status: "up_to_date", currentVersion: "" };
    }
    return {
      status: "available",
      version: update.version,
      date: update.date ?? null,
      body: update.body ?? null,
      currentVersion: update.currentVersion,
      update,
    };
  } catch (e: any) {
    return {
      status: "error",
      message: typeof e === "string" ? e : (e?.message ?? String(e)),
    };
  }
}

export interface DownloadProgress {
  downloaded: number;
  total: number | null;
}

/**
 * 利用可能な Update をダウンロード + インストールし、再起動する。
 * `onProgress` は進捗バイト数で呼ばれる（total は不明な場合 null）。
 */
export async function applyUpdate(
  update: Update,
  onProgress?: (progress: DownloadProgress) => void,
): Promise<void> {
  let downloaded = 0;
  let total: number | null = null;
  await update.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? null;
        onProgress?.({ downloaded: 0, total });
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        onProgress?.({ downloaded, total });
        break;
      case "Finished":
        onProgress?.({ downloaded, total });
        break;
    }
  });
  await relaunch();
}
