import { useEffect, useState } from "react";
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
  // 図の代替テキスト生成（Phase 8c）は画像 1 枚ごとに課金されるので、LCIR とは別の同意フラグ。
  const [altTextEnabled, setAltTextEnabled] = useState(false);
  // 代替テキスト未生成の図の件数（課金される前に規模を見せる）。
  const [altTextPending, setAltTextPending] = useState<number | null>(null);
  const refreshAltTextPending = () => {
    invoke<number>("count_figures_missing_alt_text").then(setAltTextPending).catch(() => {});
  };
  useEffect(() => {
    invoke<boolean>("get_lcir_enabled").then(setLcirEnabled).catch(() => {});
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
      const r = await invoke<{ total: number; indexed: number; needs_ocr: number; failed: number }>(
        "index_missing_attachments",
      );
      if (r.total === 0) {
        setMessage(t("settings.data.indexMissingNone"));
      } else {
        setMessage(
          t("settings.data.indexMissingDone", {
            indexed: r.indexed,
            total: r.total,
            needsOcr: r.needs_ocr,
            failed: r.failed,
          }),
        );
      }
    } catch (e) {
      setError(t("settings.data.indexMissingError", { error: errMsg(e) }));
    } finally {
      setBusy(null);
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
      const r = await invoke<{
        enabled: boolean;
        total: number;
        built: number;
        reused: number;
        failed: number;
      }>(kind === "build" ? "build_missing_lcir" : "rebuild_outdated_lcir");
      if (!r.enabled) {
        setMessage(t("settings.data.lcirDisabled"));
      } else if (r.total === 0) {
        setMessage(
          kind === "build" ? t("settings.data.lcirNone") : t("settings.data.lcirRebuildNone"),
        );
      } else {
        setMessage(
          t(kind === "build" ? "settings.data.lcirDone" : "settings.data.lcirRebuildDone", {
            total: r.total,
            built: r.built,
            reused: r.reused,
            failed: r.failed,
          }),
        );
      }
    } catch (e) {
      const s = errMsg(e);
      setError(
        s.includes("already_running")
          ? t("settings.data.lcirBatchRunning")
          : t("settings.data.lcirError", { error: s }),
      );
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

  const handleGenerateAltTexts = async () => {
    setAltTextRunning(true);
    setMessage(null);
    setError(null);
    setAltTextProgress(null);
    try {
      const r = await invoke<{
        enabled: boolean;
        total: number;
        generated: number;
        skipped: number;
        failed: number;
        aborted: boolean;
        abort_reason: string | null;
      }>("generate_vision_alt_texts");
      if (!r.enabled) {
        setMessage(t("settings.data.altTextDisabled"));
      } else if (r.total === 0) {
        setMessage(t("settings.data.altTextNone"));
      } else {
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
            : t("settings.data.altTextAborted");
        setMessage(reason ? `${done} ${reason}` : done);
      }
    } catch (e) {
      const s = errMsg(e);
      setError(
        s.includes("already_running")
          ? t("settings.data.altTextRunning")
          : t("settings.data.altTextError", { error: s }),
      );
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
      const r = await invoke<{
        total: number;
        fetched: number;
        built: number;
        pdf_only: number;
        failed: number;
      }>("fetch_missing_arxiv_sources");
      if (r.total === 0) {
        setMessage(t("settings.data.texFetchNone"));
      } else {
        setMessage(
          t("settings.data.texFetchDone", {
            total: r.total,
            fetched: r.fetched,
            built: r.built,
            pdfOnly: r.pdf_only,
            failed: r.failed,
          }),
        );
      }
    } catch (e) {
      const s = errMsg(e);
      // 多重起動ガードに弾かれたケースは専用の案内にする。
      setError(
        s.includes("already_running")
          ? t("settings.data.texFetchRunning")
          : t("settings.data.texFetchError", { error: s }),
      );
    } finally {
      setFetchTexRunning(false);
      setTexProgress(null);
      refreshAltTextPending();
    }
  };

  const handleGcConfirmed = async () => {
    setConfirmGc(false);
    setGcRunning(true);
    setMessage(null);
    setError(null);
    setGcProgress(null);
    try {
      const r = await invoke<GcOutcome>("run_lcir_gc");
      // **skip 件数を「何も無かった」に丸めない。** skip は「実行中に別経路が
      // 書き込んだので対象から外した」という別の事実で、0 件回収とは意味が違う。
      if (r.versions_removed === 0 && r.versions_tombstoned === 0 && r.versions_skipped === 0) {
        setMessage(t("settings.data.gcNone"));
      } else {
        setMessage(
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
        );
      }
    } catch (e) {
      const s = errMsg(e);
      setError(
        s.includes("already_running")
          ? t("settings.data.gcRunning")
          : t("settings.data.gcError", { error: s }),
      );
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
          <SecondaryBtn onClick={handleBackupNow} disabled={busy === "backup" || gcRunning}>
            {busy === "backup" ? t("common.loading") : t("settings.data.backupNow")}
          </SecondaryBtn>
          <SecondaryBtn onClick={handleOpenFolder}>
            {t("settings.data.openBackupFolder")}
          </SecondaryBtn>
          <SecondaryBtn
            onClick={() => setConfirmRestore(true)}
            disabled={busy === "restore" || confirmRestore || gcRunning}
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
        <SecondaryBtn onClick={handleIndexMissing} disabled={busy === "index_missing" || gcRunning}>
          {busy === "index_missing" ? t("settings.data.indexMissingBusy") : t("settings.data.indexMissing")}
        </SecondaryBtn>
      </Section>

      <Section title={t("settings.data.lcir")} description={t("settings.data.lcirDesc")}>
        <label style={{ display: "flex", alignItems: "center", gap: 10, padding: "6px 0", cursor: "pointer" }}>
          <input type="checkbox" checked={lcirEnabled} onChange={(e) => void toggleLcir(e.target.checked)} />
          <span style={{ fontSize: 12.5, color: "var(--text)" }}>{t("settings.data.lcirEnable")}</span>
        </label>
        <div style={{ marginTop: 6, display: "flex", gap: 6, flexWrap: "wrap" }}>
          <SecondaryBtn
            onClick={() => handleLcirBatch("build")}
            disabled={!lcirEnabled || lcirBatch !== null || gcRunning}
          >
            {lcirBatch === "build"
              ? lcirProgress
                ? t("settings.data.lcirBusyProgress", {
                    done: lcirProgress.done,
                    total: lcirProgress.total,
                  })
                : t("settings.data.lcirBusy")
              : t("settings.data.lcirBuild")}
          </SecondaryBtn>
          <SecondaryBtn
            onClick={() => handleLcirBatch("rebuild")}
            disabled={!lcirEnabled || lcirBatch !== null || gcRunning}
          >
            {lcirBatch === "rebuild"
              ? lcirProgress
                ? t("settings.data.lcirRebuildBusyProgress", {
                    done: lcirProgress.done,
                    total: lcirProgress.total,
                  })
                : t("settings.data.lcirRebuildBusy")
              : t("settings.data.lcirRebuild")}
          </SecondaryBtn>
          <SecondaryBtn
            onClick={handleFetchTex}
            disabled={!lcirEnabled || fetchTexRunning || lcirBatch !== null || gcRunning}
          >
            {fetchTexRunning
              ? texProgress
                ? t("settings.data.texFetchBusyProgress", {
                    done: texProgress.done,
                    total: texProgress.total,
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
            onClick={handleGenerateAltTexts}
            disabled={!lcirEnabled || !altTextEnabled || altTextRunning || gcRunning}
          >
            {altTextRunning
              ? altTextProgress
                ? t("settings.data.altTextBusyProgress", {
                    done: altTextProgress.done,
                    total: altTextProgress.total,
                  })
                : t("settings.data.altTextBusy")
              : t("settings.data.altText")}
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
            onClick={() => void refreshStorage().then(() => setConfirmGc(true))}
            disabled={
              gcRunning ||
              confirmGc ||
              busy !== null ||
              lcirBatch !== null ||
              fetchTexRunning ||
              altTextRunning ||
              // 未取得（null）のうちも押させない。押しても確認ボックスは
              // `confirmGc && storage` で出ないので、無反応のボタンになるため。
              !storage ||
              storage.gc.versions === 0
            }
          >
            {gcRunning
              ? gcProgress
                ? t("settings.data.gcBusyProgress", {
                    done: gcProgress.done,
                    total: gcProgress.total,
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
