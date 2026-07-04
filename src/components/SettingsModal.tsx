import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
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
  const [channel, setChannel] = useState<"stable" | "beta">("stable");
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

      <Section title={t("settings.updates.channel")}>
        <Segmented<"stable" | "beta">
          value={channel}
          onChange={setChannel}
          options={[
            { id: "stable", label: t("settings.updates.channelStable") },
            { id: "beta",   label: t("settings.updates.channelBeta") },
          ]}
        />
      </Section>
    </>
  );
}

function DataTab() {
  const { t } = useTranslation();
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  return (
    <>
      <Section title={t("settings.data.backup")} description={t("settings.data.backupDesc")}>
        <div style={{ display: "flex", gap: 6 }}>
          <SecondaryBtn onClick={handleBackupNow} disabled={busy === "backup"}>
            {busy === "backup" ? t("common.loading") : t("settings.data.backupNow")}
          </SecondaryBtn>
          <SecondaryBtn onClick={handleOpenFolder}>
            {t("settings.data.openBackupFolder")}
          </SecondaryBtn>
        </div>
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
        <SecondaryBtn onClick={handleIndexMissing} disabled={busy === "index_missing"}>
          {busy === "index_missing" ? t("settings.data.indexMissingBusy") : t("settings.data.indexMissing")}
        </SecondaryBtn>
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
