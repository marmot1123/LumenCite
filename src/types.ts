// 文献種別。既存の 6 種（article 等）は BibTeX 由来のキーのまま据え置き、v0.4.0 で
// 追加した種別は Zotero のアイテムタイプ名（camelCase）をそのままキーに使う。
// entry_type は DB 上は自由 TEXT（制約なし）なので、値の追加にマイグレーションは不要。
export type EntryType =
  | "article"
  | "inproceedings"
  | "preprint"
  | "book"
  | "bookSection"
  | "thesis"
  | "report"
  | "magazineArticle"
  | "newspaperArticle"
  | "encyclopediaArticle"
  | "dictionaryEntry"
  | "manuscript"
  | "presentation"
  | "patent"
  | "standard"
  | "dataset"
  | "computerProgram"
  | "webpage"
  | "misc";

/** Add/Edit シートのプルダウンに出す順序。関連する種別を近くに並べてある。 */
export const ENTRY_TYPES: EntryType[] = [
  "article",
  "inproceedings",
  "preprint",
  "book",
  "bookSection",
  "thesis",
  "report",
  "magazineArticle",
  "newspaperArticle",
  "encyclopediaArticle",
  "dictionaryEntry",
  "manuscript",
  "presentation",
  "patent",
  "standard",
  "dataset",
  "computerProgram",
  "webpage",
  "misc",
];
export type ViewMode = "table" | "covers" | "timeline" | "graph";
export type Density = "compact" | "default" | "comfortable";
export type ThemeMode = "light" | "dark" | "auto";
export type ResolvedTheme = "light" | "dark";
export type AccentName = "amber" | "indigo" | "teal" | "rose";
export type SearchScope = "meta" | "fulltext";

export interface ImportResult {
  imported: number;
  skipped: number;
  errors: string[];
}

export type HighlightColor = "yellow" | "green" | "blue";

export interface Highlight {
  id: number;
  entry_id: number;
  page: number;
  x: number;
  y: number;
  width: number;
  height: number;
  color: HighlightColor;
  text: string;
  note: string | null;
  created_at: string;
}

export interface HighlightInput {
  entry_id: number;
  page: number;
  x: number;
  y: number;
  width: number;
  height: number;
  color: HighlightColor;
  text: string;
  note?: string | null;
}

export interface HighlightUpdate {
  color?: HighlightColor;
  /** 空文字列を渡すとノートが NULL に戻る */
  note?: string;
}

/**
 * 著者の外部識別子（ORCID 以外も含む）。v0.3.0 で導入。
 * scheme は 'orcid' / 'scopus' / 'dblp' / 'semantic_scholar' / 'wikidata' /
 * 'isni' / 'viaf' / 'researcher_id' / 'google_scholar' 等の小文字キー。
 */
export interface AuthorIdentifier {
  author_id: number;
  scheme: string;
  value: string;
  url?: string | null;
}

/** バックエンド `db::authors` 由来の Author 構造体。v0.3.0 で多言語名・読み仮名・団体著者対応。 */
export interface Author {
  id: number;
  name: string;
  given_name?: string | null;
  middle_name?: string | null;
  family_name?: string | null;
  suffix?: string | null;
  name_particle?: string | null;

  /** オリジナル言語表記（漢字フルネーム等）。表示やソートの補助に使う。 */
  name_original?: string | null;
  given_name_original?: string | null;
  family_name_original?: string | null;
  /** ISO 15924 文字種コード（Hani / Hang / Cyrl / Latn / Arab ...） */
  original_script?: string | null;

  /** 五十音ソート・かな検索用 */
  reading_family?: string | null;
  reading_given?: string | null;

  /** 団体著者フラグ（IEEE / OECD 等）。true なら given/family を無視し name を literal 扱い。 */
  is_organization: boolean;

  email?: string | null;
  homepage_url?: string | null;
  notes?: string | null;

  /** 互換維持の専用カラム。識別子テーブルにも併記される。 */
  orcid?: string | null;
  updated_at?: string | null;

  /** ORCID を含む全 identifier。`get_author` / `search_authors` で JOIN して詰める。 */
  identifiers: AuthorIdentifier[];
}

/** identifiers の編集入力（url は省略可）。 */
export interface AuthorIdentifierInput {
  scheme: string;
  value: string;
  url?: string | null;
}

/**
 * 著者の新規作成 / 更新 (`update_author`) で渡す入力。
 * すべての文字列フィールドは省略可で、未指定 = null/未設定として保存される。
 */
export interface AuthorInput {
  name: string;
  given_name?: string | null;
  middle_name?: string | null;
  family_name?: string | null;
  suffix?: string | null;
  name_particle?: string | null;

  name_original?: string | null;
  given_name_original?: string | null;
  family_name_original?: string | null;
  original_script?: string | null;

  reading_family?: string | null;
  reading_given?: string | null;

  is_organization?: boolean;

  email?: string | null;
  homepage_url?: string | null;
  notes?: string | null;

  orcid?: string | null;

  identifiers?: AuthorIdentifierInput[];
}

export interface Tag {
  id: number;
  name: string;
}

export interface Collection {
  id: number;
  name: string;
  parent_id?: number;
  children: Collection[];
}

export interface Attachment {
  id: number;
  entry_id: number;
  file_name: string;
  mime_type: string;
  created_at: string;
}

export interface EntrySummary {
  id: number;
  title: string;
  year?: number;
  entry_type: EntryType;
  authors: Author[];
  tags: Tag[];
  has_attachment: boolean;
  created_at: string;
  journal?: string;
  starred: boolean;
}

export interface FulltextHit {
  entry: EntrySummary;
  attachment_id: number;
  page: number;
  snippet: string;
}

// サイドバー各行の件数。total/starred/unfiled はゴミ箱を除外、trash はゴミ箱内。
// collections/tags は id（数値）-> 件数。JSON 上は文字列キーになるので Record<string, number>。
export interface SidebarCounts {
  total: number;
  starred: number;
  unfiled: number;
  trash: number;
  collections: Record<string, number>;
  tags: Record<string, number>;
}

export interface EntryDetail extends EntrySummary {
  /** BibTeX エントリキー。null/未設定なら export 時に自動生成される */
  citation_key?: string;
  doi?: string;
  isbn?: string;
  arxiv_id?: string;
  url?: string;
  abstract_?: string;
  notes?: string;
  summary?: string;
  summary_model?: string;
  summary_generated_at?: string;
  deleted_at?: string;
  extra_fields: Record<string, string>;
  attachments: Attachment[];
  relations: {
    entry: EntrySummary;
    relation_type: string;
    direction: "from" | "to";
  }[];
  collections: Collection[];
}

// LLM 要約関連
export type LlmProvider = "openai" | "anthropic";
export type SummarySource = "abstract" | "fulltext";

export interface LlmSettings {
  provider: LlmProvider;
  model: string;
  summary_source: SummarySource;
  /** ユーザーが上書きできるシステムプロンプト。空文字なら backend の DEFAULT_SYSTEM_PROMPT が使われる */
  summary_prompt: string;
  /** OCR 用プロバイダ/モデル。未指定（null）なら provider/model にフォールバック */
  ocr_provider?: LlmProvider | null;
  ocr_model?: string | null;
}

export type SummaryStreamEvent =
  | { kind: "start"; model: string }
  | { kind: "delta"; text: string }
  | { kind: "done"; full_text: string }
  | { kind: "error"; message: string };

export interface EntryInput {
  title: string;
  year?: number;
  entry_type: EntryType;
  /** 固定 cite key。省略/空文字なら自動生成（NULL 保存） */
  citation_key?: string;
  doi?: string;
  isbn?: string;
  arxiv_id?: string;
  url?: string;
  abstract_?: string;
  notes?: string;
  extra_fields?: Record<string, string>;
  author_ids?: number[];
  author_names?: string[];
  /**
   * v0.3.0: 構造化された著者情報を直接渡すルート。
   * 指定すれば backend は `author_names` を無視してこちらを `get_or_create_author`
   * に流す。AddSheet / EditSheet の AuthorEditor 経由の入力や bibtex / CrossRef
   * 取り込みなどで使う。
   */
  authors?: AuthorInput[];
  tag_ids?: number[];
}

// ── Chat (v0.2.0) ────────────────────────────────────────────────────────────

export type ScopeMode = "all" | "entries";
export type ChatRole = "user" | "assistant" | "tool";

/** 外部 MCP サーバー設定（Claude Desktop の mcpServers 1 エントリ相当）。 */
export interface McpServerConfig {
  id: string;
  command: string;
  args: string[];
  env: Record<string, string>;
}

/** MCP サーバーの起動状態（backend の serde tag="state" と一致）。 */
export type McpServerStatus =
  | { state: "running"; tool_count: number }
  | { state: "failed"; error: string };

/** 設定済み MCP サーバー + 起動状態（list_mcp_servers の戻り値）。status が null なら状態不明。 */
export interface McpServerInfo extends McpServerConfig {
  status: McpServerStatus | null;
}

/** 公開 MCP サーバー（LumenCite 自身）の状態（get_mcp_server_status の戻り値）。 */
export interface McpServerStatusInfo {
  enabled: boolean;
  running: boolean;
  port: number;
  has_token: boolean;
  /** Phase 2: write 系ツールを公開しているか（mcp_server.write_enabled）。 */
  write_enabled: boolean;
}

/** backend の chat_sessions 行（entry_count を投影）。 */
export interface ChatSession {
  id: number;
  title: string;
  provider: string;
  model: string;
  system_prompt: string | null;
  scope_mode: ScopeMode;
  entry_count: number;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
}

/** assistant のツール呼び出し 1 件（LLM 由来）。 */
export interface ToolCallSpec {
  call_id: string;
  tool_name: string;
  /** JSON 引数 */
  arguments: unknown;
}

/** backend が返す chat_messages の生行。 */
export interface ChatMessageRow {
  id: number;
  session_id: number;
  role: ChatRole;
  content: string;
  /** assistant のツール呼び出し列の JSON 文字列 */
  tool_calls: string | null;
  tool_call_id: string | null;
  created_at: string;
  position: number;
}

export interface SessionWithMessages {
  session: ChatSession;
  messages: ChatMessageRow[];
  entry_ids: number[];
}

/** chat_send_message の Channel<ChatStreamEvent> で届くイベント。
 *  backend の serde(tag = "kind", rename_all = "snake_case") と一致させる。 */
export type ChatStreamEvent =
  | { kind: "session_started"; session_id: number }
  | { kind: "delta"; text: string }
  | { kind: "tool_call_proposed"; call_id: string; tool_name: string; args_preview: string; needs_approval: boolean }
  | { kind: "tool_call_executed"; call_id: string; result_summary: string }
  | { kind: "message_persisted"; message_id: number; role: ChatRole }
  | { kind: "done" }
  | { kind: "error"; message: string };

// ── Chat UI view models ──
export type ToolCallState = "needs_approval" | "running" | "done" | "rejected";

export interface UiToolCall {
  call_id: string;
  tool_name: string;
  /** 引数のプレビュー文字列（stream は args_preview、履歴復元時は arguments の JSON）。 */
  args_preview: string;
  needs_approval: boolean;
  state: ToolCallState;
  result_summary?: string;
}

export interface UiChatMessage {
  /** 永続化前は undefined */
  id?: number;
  role: ChatRole;
  content: string;
  tool_calls: UiToolCall[];
  /** delta を受信中の assistant メッセージか */
  streaming?: boolean;
}

/** 型固有フィールドのメタデータ。i18n キーで label / placeholder を引く。 */
export interface ExtraFieldDef {
  key: string;
  labelKey: string;
  placeholderKey?: string;
  mono?: boolean;
}

export const EXTRA_FIELDS_BY_TYPE: Record<EntryType, ExtraFieldDef[]> = {
  article: [
    { key: "journal",   labelKey: "extraField.journal",   placeholderKey: "extraFieldPlaceholder.journal" },
    { key: "volume",    labelKey: "extraField.volume",    placeholderKey: "extraFieldPlaceholder.volume" },
    { key: "issue",     labelKey: "extraField.issue",     placeholderKey: "extraFieldPlaceholder.issue" },
    { key: "pages",     labelKey: "extraField.pages",     placeholderKey: "extraFieldPlaceholder.pages" },
    { key: "publisher", labelKey: "extraField.publisher", placeholderKey: "extraFieldPlaceholder.publisher" },
  ],
  book: [
    { key: "publisher", labelKey: "extraField.publisher",  placeholderKey: "extraFieldPlaceholder.publisherBook" },
    { key: "address",   labelKey: "extraField.address",    placeholderKey: "extraFieldPlaceholder.address" },
    { key: "edition",   labelKey: "extraField.edition",    placeholderKey: "extraFieldPlaceholder.edition" },
    { key: "series",    labelKey: "extraField.series" },
    { key: "pages",     labelKey: "extraField.pagesCount", placeholderKey: "extraFieldPlaceholder.pagesCount" },
  ],
  inproceedings: [
    { key: "booktitle",    labelKey: "extraField.booktitle",    placeholderKey: "extraFieldPlaceholder.booktitle" },
    { key: "pages",        labelKey: "extraField.pages",        placeholderKey: "extraFieldPlaceholder.pagesProc" },
    { key: "publisher",    labelKey: "extraField.publisher",    placeholderKey: "extraFieldPlaceholder.publisherProc" },
    { key: "address",      labelKey: "extraField.addressEvent", placeholderKey: "extraFieldPlaceholder.addressProc" },
    { key: "organization", labelKey: "extraField.organization" },
  ],
  thesis: [
    { key: "school",  labelKey: "extraField.school",          placeholderKey: "extraFieldPlaceholder.school" },
    { key: "address", labelKey: "extraField.addressLocation", placeholderKey: "extraFieldPlaceholder.addressLocation" },
  ],
  webpage: [
    { key: "howpublished", labelKey: "extraField.howpublished", placeholderKey: "extraFieldPlaceholder.howpublished" },
  ],
  misc: [
    { key: "howpublished", labelKey: "extraField.howpublishedMisc", placeholderKey: "extraFieldPlaceholder.howpublishedMisc" },
    { key: "publisher",    labelKey: "extraField.publisherMisc" },
  ],
  // ── v0.4.0 で追加した Zotero 由来の種別 ───────────────────────────────────
  preprint: [
    { key: "repository", labelKey: "extraField.repository", placeholderKey: "extraFieldPlaceholder.repository" },
    { key: "version",    labelKey: "extraField.version",    placeholderKey: "extraFieldPlaceholder.version" },
  ],
  bookSection: [
    { key: "booktitle", labelKey: "extraField.bookTitle", placeholderKey: "extraFieldPlaceholder.bookTitle" },
    { key: "publisher", labelKey: "extraField.publisher", placeholderKey: "extraFieldPlaceholder.publisherBook" },
    { key: "address",   labelKey: "extraField.address",   placeholderKey: "extraFieldPlaceholder.address" },
    { key: "edition",   labelKey: "extraField.edition",   placeholderKey: "extraFieldPlaceholder.edition" },
    { key: "series",    labelKey: "extraField.series" },
    { key: "pages",     labelKey: "extraField.pages",     placeholderKey: "extraFieldPlaceholder.pages" },
  ],
  report: [
    { key: "institution",  labelKey: "extraField.institution",  placeholderKey: "extraFieldPlaceholder.institutionReport" },
    { key: "reportNumber", labelKey: "extraField.reportNumber", placeholderKey: "extraFieldPlaceholder.reportNumber" },
    { key: "reportType",   labelKey: "extraField.reportType",   placeholderKey: "extraFieldPlaceholder.reportType" },
    { key: "address",      labelKey: "extraField.addressLocation" },
    { key: "pages",        labelKey: "extraField.pages" },
  ],
  magazineArticle: [
    { key: "journal", labelKey: "extraField.publication", placeholderKey: "extraFieldPlaceholder.publicationMag" },
    { key: "volume",  labelKey: "extraField.volume" },
    { key: "issue",   labelKey: "extraField.issue" },
    { key: "pages",   labelKey: "extraField.pages" },
  ],
  newspaperArticle: [
    { key: "journal", labelKey: "extraField.publication", placeholderKey: "extraFieldPlaceholder.publicationNews" },
    { key: "edition", labelKey: "extraField.edition" },
    { key: "section", labelKey: "extraField.section", placeholderKey: "extraFieldPlaceholder.section" },
    { key: "pages",   labelKey: "extraField.pages" },
    { key: "address", labelKey: "extraField.addressLocation" },
  ],
  encyclopediaArticle: [
    { key: "booktitle", labelKey: "extraField.encyclopediaTitle", placeholderKey: "extraFieldPlaceholder.encyclopediaTitle" },
    { key: "publisher", labelKey: "extraField.publisher" },
    { key: "volume",    labelKey: "extraField.volume" },
    { key: "pages",     labelKey: "extraField.pages" },
    { key: "edition",   labelKey: "extraField.edition" },
  ],
  dictionaryEntry: [
    { key: "booktitle", labelKey: "extraField.dictionaryTitle", placeholderKey: "extraFieldPlaceholder.dictionaryTitle" },
    { key: "publisher", labelKey: "extraField.publisher" },
    { key: "volume",    labelKey: "extraField.volume" },
    { key: "pages",     labelKey: "extraField.pages" },
    { key: "edition",   labelKey: "extraField.edition" },
  ],
  manuscript: [
    { key: "manuscriptType", labelKey: "extraField.manuscriptType", placeholderKey: "extraFieldPlaceholder.manuscriptType" },
    { key: "address",        labelKey: "extraField.addressLocation", placeholderKey: "extraFieldPlaceholder.addressLocation" },
  ],
  presentation: [
    { key: "meetingName",      labelKey: "extraField.meetingName",      placeholderKey: "extraFieldPlaceholder.meetingName" },
    { key: "presentationType", labelKey: "extraField.presentationType", placeholderKey: "extraFieldPlaceholder.presentationType" },
    { key: "address",          labelKey: "extraField.addressEvent",     placeholderKey: "extraFieldPlaceholder.addressProc" },
  ],
  patent: [
    { key: "patentNumber", labelKey: "extraField.patentNumber", placeholderKey: "extraFieldPlaceholder.patentNumber" },
    { key: "applicant",    labelKey: "extraField.applicant",    placeholderKey: "extraFieldPlaceholder.applicant" },
    { key: "address",      labelKey: "extraField.addressLocation" },
  ],
  standard: [
    { key: "standardNumber", labelKey: "extraField.standardNumber", placeholderKey: "extraFieldPlaceholder.standardNumber" },
    { key: "organization",   labelKey: "extraField.organization",   placeholderKey: "extraFieldPlaceholder.organizationStd" },
    { key: "address",        labelKey: "extraField.addressLocation" },
  ],
  dataset: [
    { key: "repository", labelKey: "extraField.repository", placeholderKey: "extraFieldPlaceholder.repositoryData" },
    { key: "version",    labelKey: "extraField.version",    placeholderKey: "extraFieldPlaceholder.version" },
    { key: "publisher",  labelKey: "extraField.publisher" },
  ],
  computerProgram: [
    { key: "version",   labelKey: "extraField.version",   placeholderKey: "extraFieldPlaceholder.versionSw" },
    { key: "publisher", labelKey: "extraField.publisher", placeholderKey: "extraFieldPlaceholder.publisherSw" },
    { key: "address",   labelKey: "extraField.addressLocation" },
  ],
};

/** 既知の extra_field キー → i18n キー（DetailPanel 等での表示用） */
export const EXTRA_FIELD_LABEL_KEYS: Record<string, string> = {
  journal:      "extraField.journal",
  booktitle:    "extraField.booktitle",
  volume:       "extraField.volume",
  issue:        "extraField.issue",
  number:       "extraField.number",
  pages:        "extraField.pages",
  publisher:    "extraField.publisher",
  address:      "extraField.address",
  edition:      "extraField.edition",
  series:       "extraField.series",
  school:       "extraField.school",
  institution:  "extraField.institution",
  organization: "extraField.organization",
  chapter:      "extraField.chapter",
  month:        "extraField.month",
  howpublished: "extraField.howpublished",
  repository:       "extraField.repository",
  version:          "extraField.version",
  reportNumber:     "extraField.reportNumber",
  reportType:       "extraField.reportType",
  section:          "extraField.section",
  manuscriptType:   "extraField.manuscriptType",
  patentNumber:     "extraField.patentNumber",
  applicant:        "extraField.applicant",
  standardNumber:   "extraField.standardNumber",
  meetingName:      "extraField.meetingName",
  presentationType: "extraField.presentationType",
};
