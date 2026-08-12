import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { useTheme } from "../hooks/useTheme";
import { useLanguage } from "../hooks/useLanguage";
import { Icon } from "./icons";
import { checkForUpdate, applyUpdate, checkLatestRelease, type UpdateAvailable, type GithubReleaseInfo } from "../lib/updater";
import { ChatSettingsTab } from "./settings/ChatSettingsTab";
import { MODEL_PRESETS, defaultModelFor } from "../lib/models";
import LumenciteLogo from "../../design/logo-exports/lumencite.svg?url";
import type { AccentName, Density, LlmProvider, LlmSettings, SummarySource, ThemeMode } from "../types";

type TabId = "appearance" | "llm" | "chat" | "bibtex" | "updates" | "data" | "about";

const REPO_URL = "https://github.com/marmot1123/lumencite";
const SPONSORS_URL = "https://github.com/sponsors/marmot1123";
const LICENSE_URL = "https://github.com/marmot1123/lumencite/blob/main/LICENSE";

interface SettingsModalProps {
  onClose: () => void;
  onOpenBibtexSync: () => void;
  /** モーダル起動時に開くタブ（既定: appearance） */
  initialTab?: TabId;
}

// tauri.conf.json の version を実行時に取得する（ハードコードすると更新漏れする）
let cachedAppVersion = "";
function useAppVersion(): string {
  const [version, setVersion] = useState(cachedAppVersion);
  useEffect(() => {
    if (cachedAppVersion) return;
    getVersion()
      .then((v) => { cachedAppVersion = v; setVersion(v); })
      .catch(() => { /* noop */ });
  }, []);
  return version;
}

const TABS: { id: TabId; iconName: Parameters<typeof Icon>[0]["name"] }[] = [
  { id: "appearance", iconName: "sparkle" },
  { id: "llm",        iconName: "info" },
  { id: "chat",       iconName: "chat" },
  { id: "bibtex",     iconName: "sync" },
  { id: "updates",    iconName: "download" },
  { id: "data",       iconName: "library" },
  { id: "about",      iconName: "star" },
];

const ACCENT_SWATCHES: { id: AccentName; color: string; labelKey: "settings.appearance.accentAmber" | "settings.appearance.accentIndigo" | "settings.appearance.accentTeal" | "settings.appearance.accentRose" }[] = [
  { id: "amber",  color: "oklch(0.62 0.14 65)",   labelKey: "settings.appearance.accentAmber" },
  { id: "indigo", color: "oklch(0.52 0.16 270)",  labelKey: "settings.appearance.accentIndigo" },
  { id: "teal",   color: "oklch(0.55 0.10 195)",  labelKey: "settings.appearance.accentTeal" },
  { id: "rose",   color: "oklch(0.58 0.16 15)",   labelKey: "settings.appearance.accentRose" },
];


/** `lcir_storage_stats` の返り値（Rust 側は snake_case のまま出す）。 */
interface StorageStats {
  file_bytes: number;
  used_bytes: number;
  free_bytes: number;
  gc: {
    versions: number;
    versions_removable: number;
    versions_tombstoned: number;
    nodes: number;
    asset_rows: number;
    asset_bytes: number;
    alt_texts_protected: number;
    carry_refs_protected: number;
    orphan_versions_skipped: number;
  };
}

/** `run_lcir_gc` の返り値。 */
interface GcOutcome {
  versions_removed: number;
  versions_tombstoned: number;
  versions_skipped: number;
  nodes_removed: number;
  asset_rows_removed: number;
  files_trashed: number;
  fts_orphans_removed: number;
  freed_bytes: number;
}

/**
 * 長時間バッチの種別。Rust 側 `batch_status::BatchKind` の文字列と 1:1 の契約なので、
 * **片方だけ変えない**（Rust 側にも同じ注意書きがある）。
 */
type BatchKind = "build" | "rebuild" | "rederive" | "gc" | "vision_alt_text" | "tex_fetch" | "ocr";

/**
 * `lcir_batch_status` の返り値（debt-32）。**バックエンドが正本**で、フロントはこれを引く。
 *
 * これが無いと、実行状態・進捗・対象件数がこのコンポーネントのローカル state にしか無く、
 * モーダルを閉じるとアンマウントで消えていた。実害は 2 つ:
 * ①閉じている間にバッチが終わると `failed` 件数を読む手段が消える
 * ②課金操作の同意を古い件数で取る（2026-08-06 に約 4 倍の過少表示で実際に起きた）。
 */
interface BatchStatus {
  /** 今走っている種別。**複数ありうる**（バックエンドは全種別を排他にしていない）。 */
  running: BatchKind[];
  /** 種別 → 直近の進捗。実行中でないものは載らない。 */
  progress: Partial<Record<BatchKind, { done: number; total: number }>>;
  /** 直近に終わったバッチ 1 本。閉じている間に終わった結果を後から読むためのもの。 */
  last: {
    kind: BatchKind;
    /** RFC3339。同じ結果を開くたび再掲しないための識別に使う。 */
    finished_at: string;
    result: unknown;
    error: string | null;
  } | null;
}

/**
 * **一度表示した「直近の結果」を覚えておく**（モジュールスコープ ＝ アンマウントで消えない）。
 *
 * `lcir_batch_status` は読み取り専用で、読んでも結果を消さない（2 つ開いた画面のうち
 * 先に読んだ方だけが見られる、という状態を作らないため）。再掲の抑制はこちらの仕事。
 */
let lastShownBatchFinishedAt: string | null = null;

/**
 * 各バッチの戻り値。**Rust 側のコマンドの戻り値と 1:1** で、`lcir_batch_status` の
 * `last.result` にもそのまま入る。
 *
 * ⚠ ここを `Record<string, ...>` の類で緩めないこと。以前 `number & boolean & string`
 * （= `never`）にキャストしていたため、**フィールド名の綴り違いが型検査を素通りしていた**
 * （`r.pdf_only` を `r.pdfOnly` と書いても通る）。i18n に渡す値が黙って undefined になる。
 */
interface LcirBatchResult {
  enabled: boolean;
  total: number;
  built: number;
  reused: number;
  failed: number;
  skipped: number;
}
interface FulltextDeriveResult {
  total: number;
  derived: number;
  skipped_ocr: number;
  skipped_empty: number;
  skipped_existing: number;
  failed: number;
}
interface VisionAltTextResult {
  enabled: boolean;
  total: number;
  generated: number;
  skipped: number;
  /**
   * 実行中にその添付が再構築され、書き込む先の版が最新でなくなった図の数（②b の W2-4）。
   * **`skipped`（説明できなかった）と混ぜない** — 次回の実行で新版の同じ図が対象に戻るので、
   * 取りこぼしではなく先送り。
   */
  stale: number;
  failed: number;
  aborted: boolean;
  abort_reason: string | null;
}
/** `run_ocr` の結果（`batch_status.last` 経由で読む）。Rust 側 `OcrOutcome` と 1:1。 */
interface OcrResult {
  /** 課金して処理したページ数。 */
  processed: number;
  /** 本文が取れて索引に残したページ数（白紙ページは含まない）。 */
  saved: number;
  planned: number;
  stopped: boolean;
  failure: string | null;
  failed_page: number | null;
  partial: boolean;
}

interface FetchArxivSourcesResult {
  /**
   * 同意面（自動取得の同意 AND `lcir.enabled`）が開いていたか。
   * **`false` と「対象 0 件」を混ぜない** ── どちらも `total: 0` だが、
   * 前者は「取りに行っていない」、後者は「相手が居なかった」で見せる文言が違う。
   */
  enabled: boolean;
  total: number;
  fetched: number;
  built: number;
  pdf_only: number;
  failed: number;
  /** 実行中に同意が外されて打ち切ったか（v1.0.0-p3）。未処理の対象は次回そのまま拾える。 */
  aborted: boolean;
}

/** バイト数を人が読める単位にする（1 KiB = 1024 B・小数 1 桁）。 */
function formatBytes(n: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let v = Math.abs(n);
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  const s = i === 0 ? String(Math.round(v)) : v.toFixed(1);
  return `${n < 0 ? "-" : ""}${s} ${units[i]}`;
}

function Section({ title, description, children }: {
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ marginBottom: 22 }}>
      <div style={{
        fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
        textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6,
      }}>{title}</div>
      {description && (
        <div style={{ fontSize: 11.5, color: "var(--text-mute)", marginBottom: 8, lineHeight: 1.55 }}>
          {description}
        </div>
      )}
      {children}
    </div>
  );
}

function Segmented<T extends string>({ value, onChange, options }: {
  value: T;
  onChange: (v: T) => void;
  options: { id: T; label: string }[];
}) {
  return (
    <div style={{
      display: "inline-flex", padding: 2, gap: 0,
      background: "var(--surface-2)", border: "1px solid var(--border)",
      borderRadius: 6, height: 26,
    }}>
      {options.map(o => {
        const active = value === o.id;
        return (
          <button
            key={o.id}
            onClick={() => onChange(o.id)}
            style={{
              padding: "0 12px", height: 22, border: "none", borderRadius: 4,
              background: active ? "var(--surface)" : "transparent",
              color: active ? "var(--text)" : "var(--text-mute)",
              fontSize: 12, fontWeight: active ? 600 : 500, cursor: "pointer",
              boxShadow: active ? "0 1px 2px rgba(0,0,0,0.05)" : "none",
            }}
          >{o.label}</button>
        );
      })}
    </div>
  );
}

function PrimaryBtn({ onClick, children, disabled }: {
  onClick?: () => void;
  children: React.ReactNode;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      style={{
        padding: "6px 12px", borderRadius: 5,
        border: "1px solid var(--border-strong)",
        background: disabled ? "var(--surface-2)" : "var(--accent-strong)",
        color: disabled ? "var(--text-faint)" : "white",
        fontSize: 12, fontWeight: 500,
        cursor: disabled ? "not-allowed" : "pointer",
      }}
    >{children}</button>
  );
}

function SecondaryBtn({ onClick, children, disabled }: {
  onClick?: () => void;
  children: React.ReactNode;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      style={{
        padding: "6px 12px", borderRadius: 5,
        border: "1px solid var(--border-strong)",
        background: "var(--surface)",
        color: disabled ? "var(--text-faint)" : "var(--text)",
        fontSize: 12, cursor: disabled ? "not-allowed" : "pointer",
      }}
    >{children}</button>
  );
}

function AppearanceTab() {
  const { t } = useTranslation();
  const { theme, accent, density, setTheme, setAccent, setDensity } = useTheme();
  const { setting: language, setLanguage } = useLanguage();

  return (
    <>
      <Section title={t("settings.appearance.language")} description={t("settings.appearance.languageDesc")}>
        <Segmented<"ja" | "en" | "auto">
          value={language}
          onChange={setLanguage}
          options={[
            { id: "ja",   label: "日本語" },
            { id: "en",   label: "English" },
            { id: "auto", label: t("settings.appearance.languageAuto") },
          ]}
        />
      </Section>

      <Section title={t("settings.appearance.theme")} description={t("settings.appearance.themeDesc")}>
        <Segmented<ThemeMode>
          value={theme}
          onChange={setTheme}
          options={[
            { id: "light", label: t("settings.appearance.themeLight") },
            { id: "dark",  label: t("settings.appearance.themeDark") },
            { id: "auto",  label: t("settings.appearance.themeAuto") },
          ]}
        />
      </Section>

      <Section title={t("settings.appearance.accent")} description={t("settings.appearance.accentDesc")}>
        <div style={{ display: "flex", gap: 8 }}>
          {ACCENT_SWATCHES.map(s => {
            const active = accent === s.id;
            return (
              <button
                key={s.id}
                onClick={() => setAccent(s.id)}
                title={t(s.labelKey)}
                style={{
                  width: 28, height: 28, borderRadius: "50%",
                  border: active ? "2px solid var(--text)" : "2px solid transparent",
                  padding: 0, background: s.color, cursor: "pointer",
                  boxShadow: active ? "0 0 0 3px var(--surface), 0 0 0 4px var(--border-strong)" : "none",
                }}
              />
            );
          })}
        </div>
      </Section>

      <Section title={t("settings.appearance.density")} description={t("settings.appearance.densityDesc")}>
        <Segmented<Density>
          value={density}
          onChange={setDensity}
          options={[
            { id: "compact",     label: t("settings.appearance.densityCompact") },
            { id: "default",     label: t("settings.appearance.densityDefault") },
            { id: "comfortable", label: t("settings.appearance.densityComfortable") },
          ]}
        />
      </Section>
    </>
  );
}

function LlmTab() {
  const { t } = useTranslation();
  const [provider, setProvider] = useState<LlmProvider>("openai");
  const [model, setModel] = useState("");
  const [source, setSource] = useState<SummarySource>("abstract");
  const [summaryPrompt, setSummaryPrompt] = useState("");
  const [ocrProvider, setOcrProvider] = useState<"" | LlmProvider>(""); // "" = chat と同じ
  const [ocrModel, setOcrModel] = useState("");
  const [defaultPrompt, setDefaultPrompt] = useState("");
  const [hasKey, setHasKey] = useState(false);
  const [apiKeyInput, setApiKeyInput] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [testStatus, setTestStatus] = useState<"idle" | "testing" | "ok" | "error">("idle");
  const [testError, setTestError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  // 起動時: バックエンドから設定を読み込む
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [settings, defaultP] = await Promise.all([
          invoke<LlmSettings>("get_llm_settings"),
          invoke<string>("get_default_summary_prompt"),
        ]);
        if (cancelled) return;
        setProvider(settings.provider);
        setModel(settings.model);
        setSource(settings.summary_source);
        setSummaryPrompt(settings.summary_prompt);
        setOcrProvider(settings.ocr_provider ?? "");
        setOcrModel(settings.ocr_model ?? "");
        setDefaultPrompt(defaultP);
        const has = await invoke<boolean>("has_api_key", { provider: settings.provider });
        if (!cancelled) setHasKey(has);
      } finally {
        if (!cancelled) setLoaded(true);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  // provider を切り替えたら鍵の有無を再確認 + モデルのデフォルトを切替
  useEffect(() => {
    if (!loaded) return;
    invoke<boolean>("has_api_key", { provider }).then(setHasKey).catch(() => setHasKey(false));
  }, [provider, loaded]);

  const persistSettings = (next: Partial<LlmSettings>) => {
    // 現在の state を基準に next で上書き。ocr_* を必ず含めて消えないようにする。
    const payload: LlmSettings = {
      provider,
      model,
      summary_source: source,
      summary_prompt: summaryPrompt,
      ocr_provider: ocrProvider || null,
      ocr_model: ocrModel || null,
      ...next,
    };
    invoke("save_llm_settings", { settings: payload }).catch(console.error);
  };

  const handleOcrProviderChange = (next: "" | LlmProvider) => {
    setOcrProvider(next);
    persistSettings({ ocr_provider: next || null });
  };
  const handleOcrModelChange = (next: string) => {
    setOcrModel(next);
    persistSettings({ ocr_model: next || null });
  };

  const handleProviderChange = (next: LlmProvider) => {
    setProvider(next);
    // プロバイダごとにモデルセットが違うので、対応プロバイダのデフォルトモデルへ強制切替。
    // （古い OpenAI モデル名のまま Anthropic に切り替えると接続エラーになるため）
    const nextModel = defaultModelFor(next);
    setModel(nextModel);
    persistSettings({ provider: next, model: nextModel });
    setTestStatus("idle");
  };

  const handleModelChange = (next: string) => {
    setModel(next);
    persistSettings({ model: next });
    setTestStatus("idle");
  };
  const handleSourceChange = (next: SummarySource) => {
    setSource(next);
    persistSettings({ summary_source: next });
  };

  const handleSaveKey = async () => {
    const value = apiKeyInput.trim();
    if (!value) return;
    try {
      await invoke("set_api_key", { provider, key: value });
      setApiKeyInput("");
      setHasKey(true);
      setTestStatus("idle");
    } catch (e) {
      console.error(e);
    }
  };

  const handleClearKey = async () => {
    try {
      await invoke("delete_api_key", { provider });
      setHasKey(false);
      setTestStatus("idle");
    } catch (e) {
      console.error(e);
    }
  };

  const handleTest = async () => {
    setTestStatus("testing");
    setTestError(null);
    try {
      await invoke("test_llm_connection", { provider, model });
      setTestStatus("ok");
    } catch (e: any) {
      setTestStatus("error");
      setTestError(typeof e === "string" ? e : (e?.message ?? String(e)));
    }
  };

  const presets = MODEL_PRESETS[provider];
  const hasCurrentInPresets = presets.some(p => p.id === model);

  return (
    <>
      <div style={{ fontSize: 12, color: "var(--text-mute)", marginBottom: 18, lineHeight: 1.55 }}>
        {t("settings.llm.description")}
      </div>

      <Section title={t("settings.llm.provider")}>
        <Segmented<LlmProvider>
          value={provider}
          onChange={handleProviderChange}
          options={[
            { id: "openai",    label: t("settings.llm.providerOpenai") },
            { id: "anthropic", label: t("settings.llm.providerAnthropic") },
          ]}
        />
      </Section>

      <Section title={t("settings.llm.model")}>
        <select
          value={model}
          onChange={e => handleModelChange(e.target.value)}
          style={{
            width: "100%", boxSizing: "border-box",
            padding: "6px 10px", borderRadius: 5,
            border: "1px solid var(--border)",
            background: "var(--surface)", color: "var(--text)",
            fontSize: 12.5, outline: "none",
            fontFamily: "var(--mono)",
            appearance: "auto",
          }}
        >
          {!hasCurrentInPresets && model && (
            <option value={model}>{model} (custom)</option>
          )}
          {presets.map(p => (
            <option key={p.id} value={p.id}>{p.label}</option>
          ))}
        </select>
      </Section>

      <Section title={t("settings.llm.apiKey")}>
        {hasKey ? (
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <span style={{
              fontSize: 12, color: "var(--text)", fontFamily: "var(--mono)",
              padding: "5px 8px", background: "var(--surface-2)",
              border: "1px solid var(--border)", borderRadius: 5, flex: 1,
            }}>•••••••••••••••• (saved)</span>
            <SecondaryBtn onClick={handleClearKey}>{t("common.delete")}</SecondaryBtn>
          </div>
        ) : (
          <div style={{ display: "flex", gap: 6 }}>
            <input
              value={apiKeyInput}
              onChange={e => setApiKeyInput(e.target.value)}
              placeholder={t("settings.llm.apiKeyPlaceholder")}
              type={showKey ? "text" : "password"}
              style={{
                flex: 1, padding: "6px 10px", borderRadius: 5,
                border: "1px solid var(--border)",
                background: "var(--surface)", color: "var(--text)",
                fontSize: 12.5, fontFamily: showKey ? "inherit" : "var(--mono)",
                outline: "none",
              }}
            />
            <SecondaryBtn onClick={() => setShowKey(v => !v)}>
              {showKey ? t("settings.llm.apiKeyHide") : t("settings.llm.apiKeyShow")}
            </SecondaryBtn>
            <SecondaryBtn onClick={handleSaveKey} disabled={!apiKeyInput.trim()}>
              {t("common.save")}
            </SecondaryBtn>
          </div>
        )}
        <div style={{ marginTop: 8, display: "flex", gap: 8, alignItems: "center" }}>
          <SecondaryBtn onClick={handleTest} disabled={!hasKey || testStatus === "testing"}>
            {testStatus === "testing" ? t("common.loading") : t("settings.llm.test")}
          </SecondaryBtn>
          {testStatus === "ok" && (
            <span style={{ fontSize: 11.5, color: "var(--success-text)" }}>OK</span>
          )}
          {testStatus === "error" && testError && (
            <span style={{ fontSize: 11.5, color: "var(--danger-strong)" }}>{testError}</span>
          )}
        </div>
      </Section>

      <Section title={t("settings.llm.source")}>
        <Segmented<SummarySource>
          value={source}
          onChange={handleSourceChange}
          options={[
            { id: "abstract", label: t("settings.llm.sourceAbstract") },
            { id: "fulltext", label: t("settings.llm.sourceFulltext") },
          ]}
        />
      </Section>

      <Section title={t("settings.llm.ocrTitle")} description={t("settings.llm.ocrDesc")}>
        <Segmented<"" | LlmProvider>
          value={ocrProvider}
          onChange={handleOcrProviderChange}
          options={[
            { id: "", label: t("settings.llm.ocrFollow") },
            { id: "openai", label: t("settings.llm.providerOpenai") },
            { id: "anthropic", label: t("settings.llm.providerAnthropic") },
          ]}
        />
        {ocrProvider !== "" && (
          <input
            value={ocrModel}
            onChange={e => setOcrModel(e.target.value)}
            onBlur={() => handleOcrModelChange(ocrModel)}
            placeholder={t("settings.llm.ocrModelPlaceholder")}
            style={{
              marginTop: 8, width: "100%", padding: "7px 10px", borderRadius: 6,
              border: "1px solid var(--border-strong)", background: "var(--surface)",
              color: "var(--text)", fontSize: 12.5, fontFamily: "var(--mono)",
            }}
          />
        )}
      </Section>

      <Section title={t("settings.llm.systemPrompt")} description={t("settings.llm.systemPromptDesc")}>
        <textarea
          value={summaryPrompt}
          onChange={e => setSummaryPrompt(e.target.value)}
          onBlur={() => persistSettings({ summary_prompt: summaryPrompt })}
          placeholder={defaultPrompt || t("settings.llm.systemPromptPlaceholder")}
          rows={6}
          style={{
            width: "100%", boxSizing: "border-box",
            padding: "8px 10px", borderRadius: 5,
            border: "1px solid var(--border)",
            background: "var(--surface)", color: "var(--text)",
            fontSize: 12.5, lineHeight: 1.55,
            resize: "vertical", outline: "none",
            fontFamily: "inherit",
          }}
        />
        <div style={{ marginTop: 6 }}>
          <SecondaryBtn
            onClick={() => { setSummaryPrompt(""); persistSettings({ summary_prompt: "" }); }}
            disabled={!summaryPrompt.trim()}
          >
            {t("settings.llm.systemPromptReset")}
          </SecondaryBtn>
        </div>
      </Section>
    </>
  );
}

function BibtexTab({ onOpenBibtexSync }: { onOpenBibtexSync: () => void }) {
  const { t } = useTranslation();
  return (
    <>
      <div style={{ fontSize: 12, color: "var(--text-mute)", marginBottom: 14, lineHeight: 1.55 }}>
        {t("settings.bibtex.description")}
      </div>
      <PrimaryBtn onClick={onOpenBibtexSync}>{t("settings.bibtex.open")}</PrimaryBtn>
    </>
  );
}

function UpdatesTab() {
  const { t } = useTranslation();
  const appVersion = useAppVersion();
  const [status, setStatus] = useState<
    "idle" | "checking" | "up_to_date" | "available" | "notify" | "downloading" | "installing" | "error"
  >("idle");
  const [available, setAvailable] = useState<UpdateAvailable | null>(null);
  const [release, setRelease] = useState<GithubReleaseInfo | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [progress, setProgress] = useState({ downloaded: 0, total: null as number | null });

  const handleCheck = async () => {
    setStatus("checking");
    setErrorMsg(null);
    setAvailable(null);
    setRelease(null);
    // Tauri updater（macOS はアプリ内更新まで可能）と GitHub API（全 OS で新版有無だけ通知）を並行実行。
    // Windows/Linux は latest.json に自 OS エントリが無く updater が新版を見つけられないため、
    // GitHub 側を通知フォールバックとして使う（DL/インストールはせず Releases を開くだけ）。
    const [result, gh] = await Promise.all([checkForUpdate(), checkLatestRelease()]);
    if (result.status === "available") {
      // アプリ内更新が可能（主に macOS）。
      setAvailable(result);
      setStatus("available");
    } else if (gh?.isNewer) {
      // updater は新版を出せないが GitHub に新版あり → 通知のみ（Releases を開く導線）。
      setRelease(gh);
      setStatus("notify");
    } else if (result.status === "up_to_date" || gh) {
      // updater が最新、または GitHub 照会が成功して新版なし。
      setStatus("up_to_date");
    } else {
      // 両経路とも失敗（updater エラー かつ GitHub 照会も失敗）。
      setErrorMsg(t("settings.updates.checkError", { error: result.status === "error" ? result.message : "network error" }));
      setStatus("error");
    }
  };

  const openReleases = () => { if (release) void openUrl(release.htmlUrl); };

  const handleInstall = async () => {
    if (!available) return;
    setStatus("downloading");
    setProgress({ downloaded: 0, total: null });
    try {
      await applyUpdate(available.update, (p) => {
        setProgress(p);
        if (p.total != null && p.downloaded >= p.total) setStatus("installing");
      });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? String(e));
      setErrorMsg(t("settings.updates.installError", { error: msg }));
      setStatus("error");
    }
  };

  const percent = progress.total ? Math.round((progress.downloaded / progress.total) * 100) : 0;

  return (
    <>
      <Section title={t("settings.updates.title")}>
        <div style={{ fontSize: 12.5, color: "var(--text)", marginBottom: 10 }}>
          {t("settings.updates.currentVersion", { version: appVersion })}
        </div>
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <SecondaryBtn onClick={handleCheck} disabled={status === "checking" || status === "downloading" || status === "installing"}>
            {status === "checking" ? t("settings.updates.checking") : t("settings.updates.check")}
          </SecondaryBtn>
          {status === "available" && available && (
            <PrimaryBtn onClick={handleInstall}>
              {t("settings.updates.install")}
            </PrimaryBtn>
          )}
        </div>

        {status === "up_to_date" && (
          <div style={{ marginTop: 10, fontSize: 12, color: "var(--success-text)" }}>
            {t("settings.updates.upToDate")}
          </div>
        )}
        {status === "available" && available && (
          <div style={{
            marginTop: 12, padding: "10px 12px", borderRadius: 6,
            background: "var(--accent-soft)", border: "1px solid var(--accent-ring)",
          }}>
            <div style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text)" }}>
              {t("settings.updates.available", { version: available.version })}
            </div>
            {available.body && (
              <details style={{ marginTop: 6 }}>
                <summary style={{ fontSize: 11, color: "var(--text-mute)", cursor: "pointer" }}>
                  {t("settings.updates.releaseNotes")}
                </summary>
                <pre style={{
                  marginTop: 4, padding: 8, borderRadius: 4,
                  background: "var(--surface)", fontSize: 11,
                  color: "var(--text)", whiteSpace: "pre-wrap", lineHeight: 1.5,
                  maxHeight: 200, overflow: "auto",
                }}>{available.body}</pre>
              </details>
            )}
          </div>
        )}
        {status === "notify" && release && (
          <div style={{
            marginTop: 12, padding: "10px 12px", borderRadius: 6,
            background: "var(--accent-soft)", border: "1px solid var(--accent-ring)",
          }}>
            <div style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text)" }}>
              {t("settings.updates.available", { version: release.latestVersion })}
            </div>
            <div style={{ marginTop: 4, fontSize: 11.5, color: "var(--text-mute)", lineHeight: 1.5 }}>
              {t("settings.updates.notifyNote")}
            </div>
            <div style={{ marginTop: 8 }}>
              <PrimaryBtn onClick={openReleases}>{t("settings.updates.openReleases")}</PrimaryBtn>
            </div>
            {release.body && (
              <details style={{ marginTop: 8 }}>
                <summary style={{ fontSize: 11, color: "var(--text-mute)", cursor: "pointer" }}>
                  {t("settings.updates.releaseNotes")}
                </summary>
                <pre style={{
                  marginTop: 4, padding: 8, borderRadius: 4,
                  background: "var(--surface)", fontSize: 11,
                  color: "var(--text)", whiteSpace: "pre-wrap", lineHeight: 1.5,
                  maxHeight: 200, overflow: "auto",
                }}>{release.body}</pre>
              </details>
            )}
          </div>
        )}
        {status === "downloading" && (
          <div style={{ marginTop: 10, fontSize: 12, color: "var(--text-mute)" }}>
            {t("settings.updates.downloading", { percent })}
          </div>
        )}
        {status === "installing" && (
          <div style={{ marginTop: 10, fontSize: 12, color: "var(--text-mute)" }}>
            {t("settings.updates.installing")}
          </div>
        )}
        {status === "error" && errorMsg && (
          <div style={{
            marginTop: 10, padding: "8px 10px", borderRadius: 6,
            background: "oklch(0.96 0.04 25)", border: "1px solid oklch(0.85 0.08 25)",
            fontSize: 11.5, color: "oklch(0.4 0.15 25)", wordBreak: "break-all",
          }}>{errorMsg}</div>
        )}
      </Section>

      {/* 更新チャンネル（stable/beta）は配信側が未対応で、トグルしても効果が無かったため
          実装まで非表示にする（CR-028）。 */}
    </>
  );
}

function DataTab() {
  const { t, i18n } = useTranslation();
  // 桁区切りは i18next の現在言語に合わせる（引数なしの toLocaleString だと
  // 同じ文の中で言語と数値書式が別の設定源から来る）。
  const num = (n: number) => n.toLocaleString(i18n.language);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // 復元の確認はアプリ内インライン UI で行う（WKWebView では window.confirm が
  // 描画されず素通りすることがあるため）。
  const [confirmRestore, setConfirmRestore] = useState(false);
  const [lcirEnabled, setLcirEnabled] = useState(false);
  // arXiv から e-print を**自動で**取りに行ってよいか（v1.0.0-p3）。
  // `lcir.enabled` は既定 ON になったので、外部通信の同意はそこから切り離してある。
  const [texAutofetchEnabled, setTexAutofetchEnabled] = useState(false);
  // 図の代替テキスト生成（Phase 8c）は画像 1 枚ごとに課金されるので、LCIR とは別の同意フラグ。
  const [altTextEnabled, setAltTextEnabled] = useState(false);
  // 代替テキスト未生成の図の件数（課金される前に規模を見せる）。
  const [altTextPending, setAltTextPending] = useState<number | null>(null);
  const refreshAltTextPending = () => {
    invoke<number>("count_figures_missing_alt_text").then(setAltTextPending).catch(() => {});
  };
  useEffect(() => {
    invoke<boolean>("get_lcir_enabled").then(setLcirEnabled).catch(() => {});
    invoke<boolean>("get_lcir_tex_autofetch_enabled").then(setTexAutofetchEnabled).catch(() => {});
    invoke<boolean>("get_lcir_vision_alt_text_enabled").then(setAltTextEnabled).catch(() => {});
    refreshAltTextPending();
  }, []);
  // arXiv TeX 一括取得の進捗（数分かかるので「固まって見える」のを避ける）。
  // 実行中フラグは共有 `busy` と別に持つ — 数分の実行中に他の Data タブ操作が busy を
  // 書き換えても、このボタンの無効化・ラベルが誤って戻らないようにするため。
  const [fetchTexRunning, setFetchTexRunning] = useState(false);
  const [texProgress, setTexProgress] = useState<{ done: number; total: number } | null>(null);
  useEffect(() => {
    const un = listen<{ done: number; total: number }>("tex-fetch-progress", (e) =>
      setTexProgress(e.payload),
    );
    return () => {
      void un.then((f) => f());
    };
  }, []);
  // LCIR 一括構築 / 再構築の進捗（PDF 1 本ごとに pdfium 抽出 + ページレンダなので長時間）。
  // 実行中フラグは共有 busy と別に持つ（数十分の実行中に他の操作が busy を書き換えても
  // このボタンのラベル・活性が誤って戻らないようにするため）。
  const [lcirBatch, setLcirBatch] = useState<"build" | "rebuild" | null>(null);
  const [lcirProgress, setLcirProgress] = useState<{ done: number; total: number } | null>(null);
  useEffect(() => {
    const un = listen<{ done: number; total: number }>("lcir-build-progress", (e) =>
      setLcirProgress(e.payload),
    );
    return () => {
      void un.then((f) => f());
    };
  }, []);
  // 代替テキスト一括生成の進捗（図の数だけ Vision 呼び出しが走るので長時間になる）。
  const [altTextRunning, setAltTextRunning] = useState(false);
  // 押す直前に取り直した件数が画面と食い違ったときの確認（debt-32）。GC と同じインライン方式
  // （window.confirm は WKWebView で素通りすることがある）。
  const [confirmAltText, setConfirmAltText] = useState(false);
  // 件数の取り直し中。**この間もボタンを止める** ── 押してから件数が返るまでの窓で
  // もう一度押せると、2 本目の invoke が already_running で弾かれ、その finally が
  // 実行中表示を消して「課金中なのに待機状態」に見える。
  const [altTextChecking, setAltTextChecking] = useState(false);
  const [altTextProgress, setAltTextProgress] = useState<{ done: number; total: number } | null>(
    null,
  );
  useEffect(() => {
    const un = listen<{ done: number; total: number }>("vision-alt-text-progress", (e) =>
      setAltTextProgress(e.payload),
    );
    return () => {
      void un.then((f) => f());
    };
  }, []);
  // ストレージの内訳と、superseded 版の GC（v1.0.0-p4）。
  // GC は非可逆なので、確認は復元と同じインライン danger ボックスにする
  // （window.confirm は WKWebView で素通りすることがある）。
  const [storage, setStorage] = useState<StorageStats | null>(null);
  const [gcRunning, setGcRunning] = useState(false);
  const [gcProgress, setGcProgress] = useState<{ done: number; total: number } | null>(null);
  const [confirmGc, setConfirmGc] = useState(false);
  // 実測 0.06 秒の COUNT だけで組んであるので mount で引いてよい
  // （表ごとの内訳を出す dbstat は数秒かかるので使っていない）。
  const refreshStorage = async () => {
    try {
      setStorage(await invoke<StorageStats>("lcir_storage_stats"));
    } catch {
      /* 表示だけなので握りつぶす */
    }
  };
  useEffect(() => {
    void refreshStorage();
  }, []);
  useEffect(() => {
    const un = listen<{ done: number; total: number }>("lcir-gc-progress", (e) =>
      setGcProgress(e.payload),
    );
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const errMsg = (e: unknown) =>
    typeof e === "string" ? e : (e as Error)?.message ?? String(e);

  // ── 長時間バッチの状態はバックエンドが正本（debt-32）─────────────────────
  const [batchStatus, setBatchStatus] = useState<BatchStatus | null>(null);
  // **まだ画面に居るか。** 「表示済み」の印を進めてよいのは実際に描画できたときだけで、
  // アンマウント後の非同期継続で進めると**誰も見ていない結果を見せないまま捨てる**
  // （debt-32 の実害①がそのまま残る）。
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  /**
   * 終わったバッチ 1 本を文言にする。**実行時の `finally` と、開き直したときの復帰の
   * 両方がここを通る** ── 2 か所で組み立てると、片方だけ直したときに
   * 「モーダルを閉じていた場合だけ表現が違う」という直しにくいずれ方をする。
   */
  const formatBatchOutcome = (
    kind: BatchKind,
    result: unknown,
    error: string | null,
  ): { message: string | null; error: string | null } => {
    if (error !== null) {
      // 多重起動ガードに弾かれたケースは専用の案内にする。
      // キーは i18n の型が literal を要求するので、組み立てずにそのまま書く。
      const busy = error.includes("already_running");
      const text = (() => {
        switch (kind) {
          case "build":
          case "rebuild":
            return busy
              ? t("settings.data.lcirBatchRunning")
              : t("settings.data.lcirError", { error });
          case "rederive":
            return busy
              ? t("settings.data.lcirBatchRunning")
              : t("settings.data.fulltextDeriveError", { error });
          case "gc":
            return busy ? t("settings.data.gcRunning") : t("settings.data.gcError", { error });
          case "vision_alt_text":
            return busy
              ? t("settings.data.altTextRunning")
              : t("settings.data.altTextError", { error });
          case "tex_fetch":
            return busy
              ? t("settings.data.texFetchRunning")
              : t("settings.data.texFetchError", { error });
          case "ocr":
            // busy（already_running）はここに来ない ── 排他に弾かれた呼び出しは
            // `batch_status.last` に載る前に返る（batch_status.rs の FinishedBatch 参照）。
            return t("settings.data.ocrError", { error });
        }
      })();
      return { message: null, error: text };
    }
    switch (kind) {
      case "build":
      case "rebuild": {
        const r = result as LcirBatchResult;
        if (!r.enabled) return { message: t("settings.data.lcirDisabled"), error: null };
        if (r.total === 0) {
          return {
            message: t(kind === "build" ? "settings.data.lcirNone" : "settings.data.lcirRebuildNone"),
            error: null,
          };
        }
        // サマリは常に出す。pdfium を読めない配布物では大半が「着手すらしていない」ので、
        // その事実を**サマリを隠さずに**併記する（分岐を排他にすると built/failed が消える）。
        return {
          message: t(
            kind === "build" ? "settings.data.lcirDone" : "settings.data.lcirRebuildDone",
            { total: r.total, built: r.built, reused: r.reused, failed: r.failed },
          ),
          error:
            r.skipped > 0
              ? t("settings.data.lcirNoPdfium", { skipped: r.skipped, total: r.total })
              : null,
        };
      }
      case "rederive": {
        const r = result as FulltextDeriveResult;
        return {
          message:
            r.total === 0
              ? t("settings.data.fulltextDeriveNone")
              : t("settings.data.fulltextDeriveDone", {
                  derived: r.derived,
                  total: r.total,
                  skippedOcr: r.skipped_ocr,
                  skippedEmpty: r.skipped_empty,
                  failed: r.failed,
                }),
          error: null,
        };
      }
      case "vision_alt_text": {
        const r = result as VisionAltTextResult;
        if (!r.enabled) return { message: t("settings.data.altTextDisabled"), error: null };
        if (r.total === 0) return { message: t("settings.data.altTextNone"), error: null };
        const done = t("settings.data.altTextDone", {
          total: r.total,
          generated: r.generated,
          skipped: r.skipped,
          failed: r.failed,
        });
        // 打ち切られたときは理由を明示する（同意を外して止めた場合と、系統的失敗を区別）。
        const reason = !r.aborted
          ? null
          : r.abort_reason === "consent_withdrawn"
            ? t("settings.data.altTextStopped")
            // **どちらの面で止まったかを区別する。** LCIR を切って止めた人に
            // 「同意チェックが外された」と言うと嘘になる（同意は ON のまま）。
            : r.abort_reason === "lcir_disabled"
              ? t("settings.data.altTextStoppedLcir")
              : t("settings.data.altTextAborted");
        // **0 でないときだけ出す**（0 が普通なので、常時表示すると重要な値が埋もれる。
        // GC の `gcSkipped` と同じ扱い）。「説明できなかった」と別の文にするのが要点で、
        // 一緒の括弧に入れると `skipped` に混ぜたのと変わらない。
        const stale =
          r.stale > 0 ? ` ${t("settings.data.altTextStale", { stale: r.stale })}` : "";
        return { message: `${done}${stale}${reason ? ` ${reason}` : ""}`, error: null };
      }
      case "tex_fetch": {
        const r = result as FetchArxivSourcesResult;
        // 代替テキスト側（`!r.enabled` → altTextDisabled）と同型。**この分岐を
        // `total === 0` より前に置くこと** ── 後ろに置くと同意 OFF が
        // 「未取得の arXiv エントリはありません」という偽の説明になる。
        if (!r.enabled) return { message: t("settings.data.texFetchDisabled"), error: null };
        if (r.total === 0) return { message: t("settings.data.texFetchNone"), error: null };
        const done = t("settings.data.texFetchDone", {
          total: r.total,
          fetched: r.fetched,
          built: r.built,
          pdfOnly: r.pdf_only,
          failed: r.failed,
        });
        // 途中で同意を外したときは、それが理由だと明示する（黙って件数が減ると失敗に見える）。
        return {
          message: r.aborted ? `${done} ${t("settings.data.texFetchStopped")}` : done,
          error: null,
        };
      }
      case "ocr": {
        const r = result as OcrResult;
        if (r.failure) {
          return {
            message: t("settings.data.ocrDoneFailed", { saved: r.saved, error: r.failure }),
            error: null,
          };
        }
        return {
          message: r.stopped
            ? t("settings.data.ocrDoneStopped", {
                processed: r.processed,
                planned: r.planned,
                saved: r.saved,
              })
            : t("settings.data.ocrDone", { processed: r.processed, saved: r.saved }),
          error: null,
        };
      }
      case "gc": {
        const r = result as GcOutcome;
        // **skip 件数を「何も無かった」に丸めない。** skip は「実行中に別経路が
        // 書き込んだので対象から外した」という別の事実で、0 件回収とは意味が違う。
        if (r.versions_removed === 0 && r.versions_tombstoned === 0 && r.versions_skipped === 0) {
          return { message: t("settings.data.gcNone"), error: null };
        }
        return {
          message:
            t("settings.data.gcDone", {
              removed: r.versions_removed,
              tombstoned: r.versions_tombstoned,
              nodes: num(r.nodes_removed),
              freed: formatBytes(r.freed_bytes),
            }) +
            // 0 でないときだけ出す（0 が普通なので、常時表示すると重要な値が埋もれる）。
            (r.versions_skipped > 0
              ? ` ${t("settings.data.gcSkipped", { skipped: r.versions_skipped })}`
              : ""),
          error: null,
        };
      }
    }
  };

  const showBatchOutcome = (kind: BatchKind, result: unknown, error: string | null) => {
    const o = formatBatchOutcome(kind, result, error);
    setMessage(o.message);
    setError(o.error);
  };

  /**
   * バックエンドの状態を引き直し、**まだ見せていない完了結果があればその場で出す**（debt-32）。
   *
   * 「直近の結果」を消費する経路をこの 1 本に絞ってある。マウント時・ポーリング・
   * ハンドラの完了後がすべてここを通るので、
   * - 開いたまま見届けた
   * - 実行中に閉じて開き直した
   * - このパネルではない場所（詳細パネル・起動時の自動処理）で終わった
   * のどれでも同じ 1 か所が拾う。
   *
   * ⚠ **印を進めるのはマウント中だけ。** アンマウント後の非同期継続でも呼ばれうるが、
   * そこで進めると描画されないまま「表示済み」になり、開き直しても二度と出ない
   * （= debt-32 の実害①が主経路で残る。レビューで 4 レンズが独立に当てた）。
   */
  const refreshBatchStatus = async (): Promise<BatchStatus | null> => {
    let s: BatchStatus;
    try {
      s = await invoke<BatchStatus>("lcir_batch_status");
    } catch {
      return null; // 表示のためだけなので握りつぶす
    }
    if (!mountedRef.current) return s;
    setBatchStatus(s);
    const last = s.last;
    if (last && last.finished_at !== lastShownBatchFinishedAt) {
      lastShownBatchFinishedAt = last.finished_at;
      showBatchOutcome(last.kind, last.result, last.error);
    }
    return s;
  };

  // マウント時に 1 回だけ状態を引く。**リスナーの貼り直しだけでは足りない** ── 実ライブラリの
  // att37（527 頁）は 1 添付に約 8 分かかり、その間 1 通もイベントが飛ばないので、
  // 開き直した直後は「何も走っていない」ように見えてしまう。
  // 閉じている間に終わっていた結果も、`refreshBatchStatus` の中でここから拾う。
  useEffect(() => {
    void refreshBatchStatus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 開いている間は常に軽く追う。理由は 3 つあり、**どれも「マウント時に 1 回」では届かない**:
  //   ① このパネルが起動していないバッチ（詳細パネル発の代替テキスト生成・起動時の再導出・
  //      リーダー／チャット発の OCR）は invoke の解決が返って来ない。⚠ 以前は「running が
  //      非空と観測できたら追い始める」だったが、**アイドルで開いた後に始まったバッチは
  //      その最初の観測が永久に来ない** ── OCR はこの節が画面をまたいだ停止手段の本体なので、
  //      課金中に停止ボタンが出ないことに直結する。running が空でも回し続ける。
  //   ② 実行中の表示がいつまでも消えない問題（同上）。
  //   ③ **完了結果もポーリングが拾う。** 開いたまま見届けた場合も、実行中に開き直した場合も、
  //      マウント時の 1 回はまだ `last` が古いので拾えない。
  // 読むのは Mutex 1 つなので安い（2 秒に 1 回・このタブを開いている間だけ）。
  useEffect(() => {
    const id = setInterval(() => void refreshBatchStatus(), 2000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 表示に使う実行中フラグは「ローカルの楽観更新 ∪ バックエンドの正本」。
  // ローカルだけだと他所で始まったバッチを取りこぼし、バックエンドだけだと
  // 押してから最初のポーリングまでボタンが押せたままになる。
  const backendRunning = batchStatus?.running ?? [];
  const activeLcirBatch: "build" | "rebuild" | null =
    lcirBatch ??
    (backendRunning.includes("build")
      ? "build"
      : backendRunning.includes("rebuild")
        ? "rebuild"
        : null);
  // 再導出（p1）と GC は同じ `LCIR_BATCH_RUNNING` を取るので、構築系ボタンも止める。
  const anyLcirBatchRunning =
    activeLcirBatch !== null ||
    backendRunning.includes("rederive") ||
    busy === "rederive_fulltext";
  const activeGcRunning = gcRunning || backendRunning.includes("gc");
  const activeAltTextRunning = altTextRunning || backendRunning.includes("vision_alt_text");
  const activeFetchTexRunning = fetchTexRunning || backendRunning.includes("tex_fetch");
  /**
   * **LCIR 系の長時間バッチはどれか 1 本しか走らない**（ゲート ②b の W1-6）。
   *
   * ボタンごとに条件を並べ直すと、1 つだけ抜けたときに誰も気づけない ── 実際
   * 代替テキストのボタンだけ `anyLcirBatchRunning` と `activeFetchTexRunning` が抜けていて、
   * **20 分の再構築の最中に課金バッチを始められた**。5 本すべてがこの 1 つを見る。
   *
   * ⚠ **これは表示であって裁定ではない。** 本当に止めるのはバックエンドの
   * `begin_lcir_batch` / `vision_alt_text_is_blocked_by_a_build_batch` で、
   * 代替テキストと TeX 取得は詳細パネルからも起動できる（この画面の disabled は届かない）。
   */
  const anyLcirJobRunning =
    anyLcirBatchRunning || activeGcRunning || activeAltTextRunning || activeFetchTexRunning;
  // 進捗は「今届いたイベント」を優先し、無ければバックエンドのスナップショット。
  const backendProgress = batchStatus?.progress ?? {};
  const activeLcirProgress =
    lcirProgress ?? (activeLcirBatch ? backendProgress[activeLcirBatch] ?? null : null);
  const activeAltTextProgress = altTextProgress ?? backendProgress.vision_alt_text ?? null;
  const activeTexProgress = texProgress ?? backendProgress.tex_fetch ?? null;
  const activeGcProgress = gcProgress ?? backendProgress.gc ?? null;

  const handleBackupNow = async () => {
    setBusy("backup");
    setMessage(null);
    setError(null);
    try {
      const path = await invoke<string>("run_backup_now");
      setMessage(t("settings.data.backupNowDone", { path }));
    } catch (e) {
      setError(t("settings.data.backupNowError", { error: errMsg(e) }));
    } finally {
      setBusy(null);
    }
  };

  const handleOpenFolder = async () => {
    try {
      await invoke("open_backup_folder");
    } catch (e) {
      setError(errMsg(e));
    }
  };

  // インライン確認で「復元する」を押した後の本処理。
  const handleRestoreConfirmed = async () => {
    setConfirmRestore(false);
    setMessage(null);
    setError(null);
    let path: string | null = null;
    try {
      path = await invoke<string | null>("pick_backup_archive");
    } catch (e) {
      setError(errMsg(e));
      return;
    }
    if (!path) return; // ダイアログをキャンセル
    setBusy("restore");
    try {
      // ステージング（検証 + 復元前の自動バックアップ）。実際の差し替えは再起動後。
      await invoke("restore_from_archive", { path });
      setMessage(t("settings.data.restoreStaged"));
      // 反映のためアプリを再起動する。
      await relaunch();
    } catch (e) {
      setError(t("settings.data.restoreError", { error: errMsg(e) }));
      setBusy(null);
    }
  };

  const handleExport = async (cmd: "export_database_json" | "export_database_markdown" | "save_bibtex") => {
    setBusy(cmd);
    setMessage(null);
    setError(null);
    try {
      const args = cmd === "save_bibtex" ? { entryIds: null, defaultName: "lumencite.bib" } : {};
      const path = await invoke<string | null>(cmd, args);
      if (path) {
        setMessage(t("settings.data.exportDone", { path }));
      } else {
        setMessage(t("settings.data.exportCancelled"));
      }
    } catch (e) {
      setError(t("settings.data.exportError", { error: errMsg(e) }));
    } finally {
      setBusy(null);
    }
  };

  const handleIndexMissing = async () => {
    setBusy("index_missing");
    setMessage(null);
    setError(null);
    try {
      const r = await invoke<{
        total: number;
        indexed: number;
        needs_ocr: number;
        failed: number;
        skipped: number;
      }>("index_missing_attachments");
      if (r.total === 0) {
        setMessage(t("settings.data.indexMissingNone"));
      } else {
        setMessage(
          t("settings.data.indexMissingDone", {
            indexed: r.indexed,
            total: r.total,
            needsOcr: r.needs_ocr,
            failed: r.failed,
            skipped: r.skipped,
          }),
        );
      }
    } catch (e) {
      setError(t("settings.data.indexMissingError", { error: errMsg(e) }));
    } finally {
      setBusy(null);
    }
  };

  /** 全文索引を LCIR の page ノードから張り直す（v1.0.0-p1）。 */
  const handleRederiveFulltext = async () => {
    setBusy("rederive_fulltext");
    setMessage(null);
    setError(null);
    try {
      await invoke<unknown>("rederive_fulltext_from_lcir");
      await refreshBatchStatus();
    } catch (e) {
      showBatchOutcome("rederive", null, errMsg(e));
    } finally {
      setBusy(null);
    }
  };

  const toggleTexAutofetch = async (next: boolean) => {
    try {
      await invoke("set_lcir_tex_autofetch_enabled", { enabled: next });
      setTexAutofetchEnabled(next);
    } catch (e) {
      setError(errMsg(e));
    }
  };

  const toggleLcir = async (next: boolean) => {
    try {
      await invoke("set_lcir_enabled", { enabled: next });
      setLcirEnabled(next);
    } catch (e) {
      setError(errMsg(e));
    }
  };

  // 未構築の一括 LCIR 化（build）と、旧抽出器版の現行版への再構築（rebuild）。
  // 後者は既存ライブラリに新フェーズの成果（定理・参照グラフ・記号・図・表）を行き渡らせる
  // 唯一の経路で、対象が数百本になりうる。
  const handleLcirBatch = async (kind: "build" | "rebuild") => {
    setLcirBatch(kind);
    setLcirProgress(null);
    setMessage(null);
    setError(null);
    try {
      await invoke<unknown>(kind === "build" ? "build_missing_lcir" : "rebuild_outdated_lcir");
      // 表示は refreshBatchStatus が担当する（消費経路を 1 本に絞る）。
      await refreshBatchStatus();
    } catch (e) {
      showBatchOutcome(kind, null, errMsg(e));
    } finally {
      setLcirBatch(null);
      setLcirProgress(null);
      // 構築・再構築で figure + crop が増えるので、課金前に見せる件数を取り直す。
      refreshAltTextPending();
    }
  };

  const toggleAltText = async (next: boolean) => {
    try {
      await invoke("set_lcir_vision_alt_text_enabled", { enabled: next });
      setAltTextEnabled(next);
    } catch (e) {
      setError(errMsg(e));
    }
  };

  /**
   * 代替テキスト生成のボタン。**押した瞬間に対象件数を取り直す**（debt-32）。
   *
   * 課金は「押したときに画面に出ていた件数」ではなく **バックエンドがその場で引き直した
   * 件数**に対して発生する（`generate_vision_alt_texts` は `figures_missing_alt_text` を
   * 自分で引く）。2026-08-06 の #7 では、再構築の途中でモーダルを開き直したために
   * マウント時点の 89 件を表示し続け、実際の対象は 346 件だった ＝ **約 4 倍の過少表示のまま
   * 非可逆・課金操作の同意を取る**状態になっていた。
   *
   * そこで、取り直した件数が画面の数と食い違ったら**走らせずに確認へ戻す**。
   * 一致していれば同意は正しい件数に対して取れているので、そのまま実行する
   * （毎回確認を挟むと、正しく見えている場合にも 1 クリック増えるだけになる）。
   */
  const handleAltTextRequest = () => proceedAltTextIfCountMatches(altTextPending);

  /**
   * 対象件数を取り直し、`expected` と一致していれば生成へ進む。食い違っていたら
   * **走らせずに確認へ戻す**（新しい件数を見せて、もう一度押させる）。
   *
   * ⚠ **確認ボックスの「実行する」もここを通す。** 確認は「件数が動いていると分かった」
   * ときにだけ出るので、そこで取り直さないと**動いていると分かっている経路だけが
   * 再確認を素通りする**という逆立ちになる（レビューの high 指摘）。
   * 件数が動き続けるなら押すたびに確認が出るが、動く対象に課金するよりはよい。
   *
   * ⚠ **`altTextChecking` はこの関数の中で立てる。** 呼び出し側に任せると、片方の入口
   * （確認ボックスの「実行する」）だけ素通りして、件数を取り直している数百ミリ秒の間
   * ボタンが押せたままになる。そこで二度押しすると 2 本目が `already_running` で弾かれ、
   * **その `finally` が実行中表示を消して「課金中なのに待機状態」に見える。**
   */
  const proceedAltTextIfCountMatches = async (expected: number | null) => {
    if (altTextChecking || altTextRunning) return;
    setMessage(null);
    setError(null);
    setAltTextChecking(true);
    try {
      let fresh: number;
      try {
        fresh = await invoke<number>("count_figures_missing_alt_text");
      } catch (e) {
        setConfirmAltText(false);
        setError(t("settings.data.altTextError", { error: errMsg(e) }));
        return;
      }
      setAltTextPending(fresh);
      if (fresh === 0) {
        setConfirmAltText(false);
        setMessage(t("settings.data.altTextNone"));
        return;
      }
      if (expected !== fresh) {
        setConfirmAltText(true);
        return;
      }
      await runGenerateAltTexts();
    } finally {
      setAltTextChecking(false);
    }
  };

  const runGenerateAltTexts = async () => {
    setConfirmAltText(false);
    setAltTextRunning(true);
    setMessage(null);
    setError(null);
    setAltTextProgress(null);
    try {
      await invoke<unknown>("generate_vision_alt_texts");
      await refreshBatchStatus();
    } catch (e) {
      showBatchOutcome("vision_alt_text", null, errMsg(e));
    } finally {
      setAltTextRunning(false);
      setAltTextProgress(null);
      refreshAltTextPending();
    }
  };

  const handleFetchTex = async () => {
    setFetchTexRunning(true);
    setMessage(null);
    setError(null);
    setTexProgress(null);
    try {
      await invoke<unknown>("fetch_missing_arxiv_sources");
      await refreshBatchStatus();
    } catch (e) {
      // 多重起動ガードに弾かれたケースは専用の案内にする（formatBatchOutcome の担当）。
      showBatchOutcome("tex_fetch", null, errMsg(e));
    } finally {
      setFetchTexRunning(false);
      setTexProgress(null);
      refreshAltTextPending();
    }
  };

  /**
   * GC の確認を開く。**開く直前に見積りを取り直す**（debt-32）── 確認ボックスに出る
   * 「回収できる版・ノード・バイト数」はマウント時に引いた値で、その後の構築・再構築・
   * バックフィルで動く。非可逆な操作の同意を古い数字で取らないための取り直し。
   */
  const handleGcRequest = async () => {
    setMessage(null);
    setError(null);
    await refreshStorage();
    setConfirmGc(true);
  };

  const handleGcConfirmed = async () => {
    setConfirmGc(false);
    setGcRunning(true);
    setMessage(null);
    setError(null);
    setGcProgress(null);
    try {
      await invoke<GcOutcome>("run_lcir_gc");
      await refreshBatchStatus();
    } catch (e) {
      showBatchOutcome("gc", null, errMsg(e));
    } finally {
      setGcRunning(false);
      setGcProgress(null);
      void refreshStorage();
    }
  };

  return (
    <>
      <Section title={t("settings.data.backup")} description={t("settings.data.backupDesc")}>
        <div style={{ display: "flex", gap: 6 }}>
          <SecondaryBtn onClick={handleBackupNow} disabled={busy === "backup" || activeGcRunning}>
            {busy === "backup" ? t("common.loading") : t("settings.data.backupNow")}
          </SecondaryBtn>
          <SecondaryBtn onClick={handleOpenFolder}>
            {t("settings.data.openBackupFolder")}
          </SecondaryBtn>
          <SecondaryBtn
            onClick={() => setConfirmRestore(true)}
            disabled={busy === "restore" || confirmRestore || activeGcRunning}
          >
            {busy === "restore" ? t("common.loading") : t("settings.data.restore")}
          </SecondaryBtn>
        </div>
        <div style={{ fontSize: 11, color: "var(--text-mute)", marginTop: 6 }}>
          {t("settings.data.restoreDesc")}
        </div>
        {confirmRestore && (
          <div style={{
            padding: "10px 12px", borderRadius: 6, marginTop: 8,
            background: "var(--danger-bg)", border: "1px solid var(--danger-border)",
          }}>
            <div style={{ fontSize: 12, color: "var(--text)", lineHeight: 1.55, marginBottom: 8 }}>
              {t("settings.data.restoreConfirm")}
            </div>
            <div style={{ display: "flex", gap: 6 }}>
              <SecondaryBtn onClick={() => setConfirmRestore(false)}>
                {t("common.cancel")}
              </SecondaryBtn>
              <SecondaryBtn onClick={handleRestoreConfirmed}>
                {t("settings.data.restoreProceed")}
              </SecondaryBtn>
            </div>
          </div>
        )}
      </Section>

      <Section title={t("settings.data.export")} description={t("settings.data.exportDesc")}>
        <div style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: "flex-start" }}>
          <SecondaryBtn onClick={() => handleExport("save_bibtex")} disabled={busy === "save_bibtex"}>
            {t("settings.data.exportBibtex")}
          </SecondaryBtn>
          <SecondaryBtn onClick={() => handleExport("export_database_json")} disabled={busy === "export_database_json"}>
            {t("settings.data.exportJson")}
          </SecondaryBtn>
          <SecondaryBtn onClick={() => handleExport("export_database_markdown")} disabled={busy === "export_database_markdown"}>
            {t("settings.data.exportMarkdown")}
          </SecondaryBtn>
        </div>
      </Section>

      <Section title={t("settings.data.fulltext")} description={t("settings.data.fulltextDesc")}>
        <SecondaryBtn onClick={handleIndexMissing} disabled={busy === "index_missing" || activeGcRunning}>
          {busy === "index_missing" ? t("settings.data.indexMissingBusy") : t("settings.data.indexMissing")}
        </SecondaryBtn>
      </Section>

      {/* **OCR は起動口がこの画面の外（リーダー／チャット）にしかない。**
          それでもここに出すのは、走っていることと止める手段を**画面をまたいで**
          持たせるため ── リーダーを離れると停止できなくなっていた（PR-1b のレビュー指摘）。 */}
      {batchStatus?.running.includes("ocr") && (
        <Section title={t("settings.data.ocrRunningTitle")} description={t("settings.data.ocrRunningDesc")}>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <span style={{ fontSize: 12.5 }}>
              {batchStatus.progress.ocr
                ? t("settings.data.ocrRunningProgress", {
                    done: batchStatus.progress.ocr.done,
                    total: batchStatus.progress.ocr.total,
                  })
                : t("settings.data.ocrRunningNoProgress")}
            </span>
            <SecondaryBtn onClick={() => { void invoke("cancel_ocr"); }}>
              {t("settings.data.ocrStop")}
            </SecondaryBtn>
          </div>
        </Section>
      )}

      <Section title={t("settings.data.lcir")} description={t("settings.data.lcirDesc")}>
        <label style={{ display: "flex", alignItems: "center", gap: 10, padding: "6px 0", cursor: "pointer" }}>
          <input type="checkbox" checked={lcirEnabled} onChange={(e) => void toggleLcir(e.target.checked)} />
          <span style={{ fontSize: 12.5, color: "var(--text)" }}>{t("settings.data.lcirEnable")}</span>
        </label>
        {/* e-print 自動取得は独立の同意面（v1.0.0-p3）。LCIR は手元の計算だが、こちらは
            相手のサーバへ数 MB を取りに行くので、既定 ON にした LCIR に含めない。 */}
        <label style={{ display: "flex", alignItems: "center", gap: 10, padding: "6px 0", cursor: "pointer" }}>
          <input
            type="checkbox"
            checked={texAutofetchEnabled}
            disabled={!lcirEnabled}
            onChange={(e) => void toggleTexAutofetch(e.target.checked)}
          />
          <span style={{ fontSize: 12.5, color: "var(--text)" }}>{t("settings.data.texAutofetchEnable")}</span>
        </label>
        <div style={{ fontSize: 11, color: "var(--text-mute)", marginTop: 4, lineHeight: 1.55 }}>
          {t("settings.data.texAutofetchDesc")}
        </div>
        <div style={{ marginTop: 6, display: "flex", gap: 6, flexWrap: "wrap" }}>
          <SecondaryBtn
            onClick={() => handleLcirBatch("build")}
            disabled={!lcirEnabled || anyLcirJobRunning}
          >
            {activeLcirBatch === "build"
              ? activeLcirProgress
                ? t("settings.data.lcirBusyProgress", {
                    done: activeLcirProgress.done,
                    total: activeLcirProgress.total,
                  })
                : t("settings.data.lcirBusy")
              : t("settings.data.lcirBuild")}
          </SecondaryBtn>
          <SecondaryBtn
            onClick={() => handleLcirBatch("rebuild")}
            disabled={!lcirEnabled || anyLcirJobRunning}
          >
            {activeLcirBatch === "rebuild"
              ? activeLcirProgress
                ? t("settings.data.lcirRebuildBusyProgress", {
                    done: activeLcirProgress.done,
                    total: activeLcirProgress.total,
                  })
                : t("settings.data.lcirRebuildBusy")
              : t("settings.data.lcirRebuild")}
          </SecondaryBtn>
          <SecondaryBtn
            onClick={handleFetchTex}
            // **`!lcirEnabled` を落とさないこと**（ゲート ②b の F-1）。同意チェックは
            // `disabled={!lcirEnabled}` で固まるので、LCIR を切ると「同意 ON のまま
            // 押せるボタン」だけが残る。バックエンドは AND で弾くので通信は起きないが、
            // 押しても何も起きないボタンになる。代替テキスト側と同型にする。
            disabled={!lcirEnabled || !texAutofetchEnabled || anyLcirJobRunning}
          >
            {activeFetchTexRunning
              ? activeTexProgress
                ? t("settings.data.texFetchBusyProgress", {
                    done: activeTexProgress.done,
                    total: activeTexProgress.total,
                  })
                : t("settings.data.texFetchBusy")
              : t("settings.data.texFetch")}
          </SecondaryBtn>
        </div>
        {/* 図の代替テキスト生成（Phase 8c）: 画像ごとに外部 API 課金が発生するため、
            LCIR とは独立のチェックボックスで明示同意を取る。 */}
        <label style={{ display: "flex", alignItems: "center", gap: 10, padding: "10px 0 0", cursor: "pointer" }}>
          <input
            type="checkbox"
            checked={altTextEnabled}
            disabled={!lcirEnabled}
            onChange={(e) => void toggleAltText(e.target.checked)}
          />
          <span style={{ fontSize: 12.5, color: "var(--text)" }}>{t("settings.data.altTextEnable")}</span>
        </label>
        <div style={{ fontSize: 11, color: "var(--text-mute)", marginTop: 4, lineHeight: 1.55 }}>
          {t("settings.data.altTextConsent")}
          {altTextPending !== null && (
            <> {t("settings.data.altTextPending", { pending: altTextPending })}</>
          )}
        </div>
        <div style={{ marginTop: 6 }}>
          <SecondaryBtn
            onClick={handleAltTextRequest}
            // **`anyLcirJobRunning` を分解して並べ直さないこと**（ゲート ②b の W1-6）。
            // ここだけ構築系と TeX 取得が抜けていて、再構築の最中に課金を始められた。
            disabled={
              !lcirEnabled ||
              !altTextEnabled ||
              anyLcirJobRunning ||
              confirmAltText ||
              altTextChecking
            }
          >
            {activeAltTextRunning
              ? activeAltTextProgress
                ? t("settings.data.altTextBusyProgress", {
                    done: activeAltTextProgress.done,
                    total: activeAltTextProgress.total,
                  })
                : t("settings.data.altTextBusy")
              : t("settings.data.altText")}
          </SecondaryBtn>
        </div>
        {/* 押す直前に取り直した件数が画面と食い違ったときだけ確認を挟む（debt-32）。
            課金は「表示していた件数」ではなく、バックエンドがその場で引き直した件数に
            対して発生するので、ずれたまま同意を取らない。 */}
        {confirmAltText && altTextPending !== null && (
          <div style={{
            padding: "10px 12px", borderRadius: 6, marginTop: 8,
            background: "var(--danger-bg)", border: "1px solid var(--danger-border)",
          }}>
            <div style={{ fontSize: 12, color: "var(--text)", lineHeight: 1.55, marginBottom: 8 }}>
              {t("settings.data.altTextCountChanged", { pending: altTextPending })}
            </div>
            <div style={{ display: "flex", gap: 6 }}>
              <SecondaryBtn onClick={() => setConfirmAltText(false)}>
                {t("common.cancel")}
              </SecondaryBtn>
              <SecondaryBtn
                onClick={() => void proceedAltTextIfCountMatches(altTextPending)}
                disabled={altTextChecking || activeAltTextRunning}
              >
                {t("settings.data.altTextProceed")}
              </SecondaryBtn>
            </div>
          </div>
        )}
        {/* 全文索引の再導出（v1.0.0-p1）。pdfium を使わない純 SQL なので秒オーダーで、
            進捗イベントは出さない。OCR 由来の索引と本文が空の添付は触らない。 */}
        <div style={{ fontSize: 11, color: "var(--text-mute)", marginTop: 10, lineHeight: 1.55 }}>
          {t("settings.data.fulltextDeriveDesc")}
        </div>
        <div style={{ marginTop: 6 }}>
          <SecondaryBtn
            onClick={handleRederiveFulltext}
            disabled={!lcirEnabled || busy !== null || anyLcirJobRunning}
          >
            {busy === "rederive_fulltext"
              ? t("settings.data.fulltextDeriveBusy")
              : t("settings.data.fulltextDerive")}
          </SecondaryBtn>
        </div>
      </Section>

      {/* ストレージ（v1.0.0-p4）。**LCIR Section の内側に入れない** —
          `lcir.enabled` を切った人ほど「もう要らない旧版が場所を食っている」を
          見たいので、GC と容量表示はフラグで gate しない。 */}
      <Section title={t("settings.data.storage")} description={t("settings.data.storageDesc")}>
        {storage && (
          <div style={{ fontSize: 11.5, color: "var(--text-mute)", lineHeight: 1.7 }}>
            <div>
              {t("settings.data.storageSize", {
                file: formatBytes(storage.file_bytes),
                used: formatBytes(storage.used_bytes),
                free: formatBytes(storage.free_bytes),
              })}
            </div>
            {storage.gc.versions > 0 ? (
              <div>
                {t("settings.data.storageReclaimable", {
                  versions: storage.gc.versions,
                  nodes: num(storage.gc.nodes),
                })}
                {storage.gc.versions_tombstoned > 0 && (
                  <> {t("settings.data.storageTombstone", {
                    tombstoned: storage.gc.versions_tombstoned,
                  })}</>
                )}
              </div>
            ) : (
              <div>{t("settings.data.storageNothingToReclaim")}</div>
            )}
            {/* 安全述語。0 が正常なので、0 でないときだけ出す。 */}
            {storage.gc.alt_texts_protected > 0 && (
              <div>
                {t("settings.data.storageProtected", {
                  altTexts: storage.gc.alt_texts_protected,
                })}
              </div>
            )}
          </div>
        )}
        <div style={{ marginTop: 8 }}>
          <SecondaryBtn
            // **確認ボックスを開く直前に見積りを取り直す。** 同じタブに再構築ボタンが
            // 並んでおり、mount 時の数字のまま非可逆な削除の同意を取ると
            // 「145 件消します」と言って別の件数を消すことになる。
            onClick={handleGcRequest}
            disabled={
              anyLcirJobRunning ||
              confirmGc ||
              busy !== null ||
              // 未取得（null）のうちも押させない。押しても確認ボックスは
              // `confirmGc && storage` で出ないので、無反応のボタンになるため。
              !storage ||
              storage.gc.versions === 0
            }
          >
            {activeGcRunning
              ? activeGcProgress
                ? t("settings.data.gcBusyProgress", {
                    done: activeGcProgress.done,
                    total: activeGcProgress.total,
                  })
                : t("settings.data.gcBusy")
              : t("settings.data.gc")}
          </SecondaryBtn>
        </div>
        {confirmGc && storage && (
          <div style={{
            padding: "10px 12px", borderRadius: 6, marginTop: 8,
            background: "var(--danger-bg)", border: "1px solid var(--danger-border)",
          }}>
            {/* **予告は行数で出す。** GC 前に「何 MB 空く」は按分推定しか作れず、
                実測で 6.5% 上振れした（free page になるのは丸ごと空いたページだけ）。
                バイトは実行後に freelist の実測差分として報告する。 */}
            <div style={{ fontSize: 12, color: "var(--text)", lineHeight: 1.55, marginBottom: 8 }}>
              {t("settings.data.gcConfirm", {
                versions: storage.gc.versions,
                nodes: num(storage.gc.nodes),
              })}
              {/* trash 送りになる crop は非可逆の主な実体なので、0 でなければ必ず出す。 */}
              {storage.gc.asset_rows > 0 && (
                <> {t("settings.data.gcConfirmCrops", {
                  crops: num(storage.gc.asset_rows),
                  size: formatBytes(storage.gc.asset_bytes),
                })}</>
              )}
            </div>
            <div style={{ display: "flex", gap: 6 }}>
              <SecondaryBtn onClick={() => setConfirmGc(false)}>{t("common.cancel")}</SecondaryBtn>
              <SecondaryBtn onClick={handleGcConfirmed}>
                {t("settings.data.gcProceed")}
              </SecondaryBtn>
            </div>
          </div>
        )}
      </Section>

      {message && (
        <div style={{
          padding: "8px 10px", borderRadius: 6, marginTop: 4,
          background: "var(--accent-soft)", border: "1px solid var(--accent-ring)",
          fontSize: 11.5, color: "var(--text)", wordBreak: "break-all",
        }}>{message}</div>
      )}
      {error && (
        <div style={{
          padding: "8px 10px", borderRadius: 6, marginTop: 4,
          background: "var(--danger-bg)", border: "1px solid var(--danger-border)",
          fontSize: 11.5, color: "var(--danger-text)", wordBreak: "break-all",
        }}>{error}</div>
      )}
    </>
  );
}

function AboutTab() {
  const { t } = useTranslation();
  const appVersion = useAppVersion();
  const open = (url: string) => { void openUrl(url); };
  return (
    <>
      <div style={{
        display: "flex", alignItems: "center", gap: 12, marginBottom: 18,
      }}>
        <img src={LumenciteLogo} alt="LumenCite" width={48} height={48} style={{ display: "block" }} />
        <div>
          <div style={{ fontSize: 16, fontWeight: 600, color: "var(--text)", letterSpacing: "-0.01em" }}>
            {t("settings.about.appTitle")}
          </div>
          <div style={{ fontSize: 12, color: "var(--text-mute)", marginTop: 2 }}>
            {t("settings.about.tagline")}
          </div>
        </div>
      </div>

      <Section title={t("settings.about.appTitle")}>
        <div style={{ fontSize: 12.5, color: "var(--text)", marginBottom: 4 }}>
          {t("settings.about.version", { version: appVersion })}
        </div>
        <div style={{ fontSize: 12.5, color: "var(--text)", marginBottom: 10 }}>
          {t("settings.about.license")}
        </div>
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
          <SecondaryBtn onClick={() => open(REPO_URL)}>{t("settings.about.openRepo")}</SecondaryBtn>
          <SecondaryBtn onClick={() => open(LICENSE_URL)}>{t("settings.about.openLicense")}</SecondaryBtn>
        </div>
      </Section>

      <Section title={t("settings.about.supportTitle")} description={t("settings.about.supportBody")}>
        <PrimaryBtn onClick={() => open(SPONSORS_URL)}>
          {t("settings.about.openSponsors")}
        </PrimaryBtn>
      </Section>

      <Section title={t("settings.about.thanksTitle")}>
        <div style={{ fontSize: 12, color: "var(--text-mute)", lineHeight: 1.6 }}>
          {t("settings.about.thanksBody")}
        </div>
      </Section>
    </>
  );
}

export function SettingsModal({ onClose, onOpenBibtexSync, initialTab }: SettingsModalProps) {
  const { t } = useTranslation();
  const [active, setActive] = useState<TabId>(initialTab ?? "appearance");

  // モーダルが開いたまま（例: アプリメニューの About）でもタブ指定に追従する
  useEffect(() => {
    if (initialTab) setActive(initialTab);
  }, [initialTab]);

  return (
    <div
      onClick={onClose}
      style={{
        position: "fixed", inset: 0,
        background: "rgba(0,0,0,0.30)",
        display: "flex", alignItems: "center", justifyContent: "center",
        zIndex: 1000,
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 760, maxWidth: "92vw", height: 540, maxHeight: "86vh",
          background: "var(--surface)",
          border: "1px solid var(--border-strong)",
          borderRadius: 10,
          boxShadow: "0 12px 32px rgba(0,0,0,0.18)",
          display: "flex", flexDirection: "column",
          overflow: "hidden",
        }}
      >
        <div style={{
          display: "flex", alignItems: "center",
          padding: "14px 18px",
          borderBottom: "1px solid var(--border)",
          background: "var(--surface)",
        }}>
          <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text)", flex: 1 }}>
            {t("settings.title")}
          </div>
          <button
            onClick={onClose}
            aria-label={t("common.close")}
            style={{
              width: 26, height: 26, padding: 0, border: "none",
              background: "transparent", borderRadius: 5, cursor: "pointer",
              display: "inline-flex", alignItems: "center", justifyContent: "center",
            }}
          >
            <Icon name="close" size={14} color="var(--text-mute)" />
          </button>
        </div>

        <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
          <nav style={{
            width: 184, flexShrink: 0,
            borderRight: "1px solid var(--border)",
            background: "var(--surface-2)",
            padding: "10px 6px",
            display: "flex", flexDirection: "column", gap: 1,
          }}>
            {TABS.map(tab => {
              const isActive = active === tab.id;
              return (
                <button
                  key={tab.id}
                  onClick={() => setActive(tab.id)}
                  style={{
                    display: "flex", alignItems: "center", gap: 8,
                    padding: "7px 10px", borderRadius: 6,
                    border: "none", background: isActive ? "var(--surface)" : "transparent",
                    color: isActive ? "var(--text)" : "var(--text-mute)",
                    fontSize: 12.5, fontWeight: isActive ? 600 : 500,
                    cursor: "pointer", textAlign: "left",
                    boxShadow: isActive ? "0 1px 2px rgba(0,0,0,0.04)" : "none",
                  }}
                >
                  <Icon name={tab.iconName} size={13} color={isActive ? "var(--text)" : "var(--text-mute)"} />
                  {t(`settings.nav.${tab.id}`)}
                </button>
              );
            })}
          </nav>

          <div style={{
            flex: 1, padding: "20px 24px",
            overflow: "auto", background: "var(--surface)",
          }}>
            {active === "appearance" && <AppearanceTab />}
            {active === "llm" && <LlmTab />}
            {active === "chat" && <ChatSettingsTab />}
            {active === "bibtex" && <BibtexTab onOpenBibtexSync={onOpenBibtexSync} />}
            {active === "updates" && <UpdatesTab />}
            {active === "data" && <DataTab />}
            {active === "about" && <AboutTab />}
          </div>
        </div>

        <div style={{
          display: "flex", justifyContent: "flex-end",
          padding: "12px 18px", borderTop: "1px solid var(--border)",
          background: "var(--surface)",
        }}>
          <SecondaryBtn onClick={onClose}>{t("common.close")}</SecondaryBtn>
        </div>
      </div>
    </div>
  );
}
