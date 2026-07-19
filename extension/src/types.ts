// バックエンド API_SPEC.md の「Web クリッパー」節と対応する型定義。

/** POST /clipper に送るペイロード（ページから抽出）。 */
export interface ClipPayload {
  url: string;
  title?: string;
  doi?: string;
  arxiv_id?: string;
  isbn?: string;
  pdf_url?: string;
  published_date?: string;
  site_name?: string;
  /** citation_author から抽出した著者名（"Given Family" 形式）。フォールバック用。 */
  authors?: string[];
}

/** POST /clipper の 200/4xx/5xx JSON 応答。 */
export interface ClipResponse {
  status: "created" | "duplicate" | "error";
  entry_id?: number;
  title?: string;
  /** created かつ PDF ダウンロードを開始したとき "downloading" */
  pdf?: string;
  /** 重複時、欠落があり初回確認を要する（アプリ側設定が未設定）。["pdf","tex"] の部分集合。 */
  confirm_missing?: string[];
  /** 重複時、設定 "1" で自動補完を開始した欠落。["pdf","tex"] の部分集合。 */
  completing?: string[];
  code?: string;
  message?: string;
}

/** POST /clipper/complete の JSON 応答。 */
export interface CompleteResponse {
  status: "completing" | "error";
  entry_id?: number;
  completing?: string[];
  remembered?: boolean;
  code?: string;
  message?: string;
}

/** confirm ポップアップに渡す保留中の欠落補完（chrome.storage.session に置く）。 */
export interface PendingMissing {
  entry_id: number;
  title: string;
  /** ["pdf"] / ["tex"] / ["pdf","tex"] のいずれか。 */
  missing: string[];
}

/** 接続コード（lc1.…）から復元した接続設定。chrome.storage.local に保存する。 */
export interface ClipperConfig {
  port: number;
  token: string;
}
