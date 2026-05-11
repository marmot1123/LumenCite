export type EntryType = "article" | "book" | "inproceedings" | "thesis" | "webpage" | "misc";
export type ViewMode = "table" | "covers" | "timeline" | "graph";
export type Density = "compact" | "default" | "comfortable";
export type ThemeMode = "light" | "dark";
export type AccentName = "amber" | "indigo" | "teal" | "rose";
export type SearchScope = "meta" | "fulltext";

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

// 型固有フィールドのメタデータ。BibTeX のフィールド名をキーに使う。
export interface ExtraFieldDef {
  key: string;
  label: string;
  placeholder?: string;
  mono?: boolean;
}

export const EXTRA_FIELDS_BY_TYPE: Record<EntryType, ExtraFieldDef[]> = {
  article: [
    { key: "journal",   label: "雑誌名",   placeholder: "Nature, NeurIPS, …" },
    { key: "volume",    label: "巻",       placeholder: "612" },
    { key: "issue",     label: "号",       placeholder: "7940" },
    { key: "pages",     label: "ページ",   placeholder: "150-160" },
    { key: "publisher", label: "出版社",   placeholder: "Springer Nature" },
  ],
  book: [
    { key: "publisher", label: "出版社",   placeholder: "MIT Press" },
    { key: "address",   label: "出版地",   placeholder: "Cambridge, MA" },
    { key: "edition",   label: "版",       placeholder: "3rd" },
    { key: "series",    label: "シリーズ", placeholder: "" },
    { key: "pages",     label: "ページ数", placeholder: "1312" },
  ],
  inproceedings: [
    { key: "booktitle",    label: "会議名／論文集名", placeholder: "Proceedings of CVPR 2024" },
    { key: "pages",        label: "ページ",          placeholder: "1234-1245" },
    { key: "publisher",    label: "出版社",          placeholder: "IEEE" },
    { key: "address",      label: "開催地",          placeholder: "Seattle, WA" },
    { key: "organization", label: "主催",            placeholder: "" },
  ],
  thesis: [
    { key: "school",  label: "大学・研究機関", placeholder: "The University of Tokyo" },
    { key: "address", label: "所在地",         placeholder: "Tokyo, Japan" },
  ],
  webpage: [
    { key: "howpublished", label: "掲載元", placeholder: "Blog post / GitHub README 等" },
  ],
  misc: [
    { key: "howpublished", label: "掲載形態", placeholder: "Technical report 等" },
    { key: "publisher",    label: "発行元",   placeholder: "" },
  ],
};

// 既知の extra_field キー → 日本語ラベル（DetailPanel での表示用）
export const EXTRA_FIELD_LABELS: Record<string, string> = {
  journal:      "雑誌名",
  booktitle:    "会議名／論文集名",
  volume:       "巻",
  issue:        "号",
  number:       "号",
  pages:        "ページ",
  publisher:    "出版社",
  address:      "出版地",
  edition:      "版",
  series:       "シリーズ",
  school:       "大学・研究機関",
  institution:  "所属機関",
  organization: "主催",
  chapter:      "章",
  month:        "月",
  howpublished: "掲載元",
};
