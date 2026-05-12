export type EntryType = "article" | "book" | "inproceedings" | "thesis" | "webpage" | "misc";
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

export interface Author {
  id: number;
  name: string;
  given_name?: string;
  family_name?: string;
  orcid?: string;
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
  doi?: string;
  isbn?: string;
  arxiv_id?: string;
  url?: string;
  abstract_?: string;
  notes?: string;
  extra_fields?: Record<string, string>;
  author_ids?: number[];
  author_names?: string[];
  tag_ids?: number[];
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
};
