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
  code?: string;
  message?: string;
}

/** 接続コード（lc1.…）から復元した接続設定。chrome.storage.local に保存する。 */
export interface ClipperConfig {
  port: number;
  token: string;
}
