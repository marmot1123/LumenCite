import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke, Channel } from "@tauri-apps/api/core";
import { Icon } from "../icons";
import { MathMarkdown } from "../MathMarkdown";
import type { EntryDetail, LlmSettings, SummarySource, SummaryStreamEvent } from "../../types";

interface SummarySheetProps {
  entry: EntryDetail;
  onClose: () => void;
  onSavedToNotes: (newNotes: string) => Promise<void> | void;
  onOpenSettings: () => void;
}

type Status = "loading" | "no_key" | "streaming" | "done" | "error";

export function SummarySheet({ entry, onClose, onSavedToNotes, onOpenSettings }: SummarySheetProps) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<Status>("loading");
  const [model, setModel] = useState<string>("");
  const [source, setSource] = useState<SummarySource>("abstract");
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [generationKey, setGenerationKey] = useState(0);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const channelRef = useRef<Channel<SummaryStreamEvent> | null>(null);

  // 生成キックオフ: マウント時と「再生成」時
  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setText("");
    setError(null);

    (async () => {
      try {
        const settings = await invoke<LlmSettings>("get_llm_settings");
        if (cancelled) return;
        const has = await invoke<boolean>("has_api_key", { provider: settings.provider });
        if (cancelled) return;
        if (!has) {
          setStatus("no_key");
          return;
        }
        setSource(settings.summary_source);

        const channel = new Channel<SummaryStreamEvent>();
        channelRef.current = channel;
        channel.onmessage = (event) => {
          if (cancelled) return;
          switch (event.kind) {
            case "start":
              setModel(event.model);
              setStatus("streaming");
              break;
            case "delta":
              setText(prev => prev + event.text);
              break;
            case "done":
              setStatus("done");
              break;
            case "error":
              setError(event.message);
              setStatus("error");
              break;
          }
        };

        await invoke("generate_summary", {
          entryId: entry.id,
          source: settings.summary_source,
          channel,
        }).catch((e: unknown) => {
          if (cancelled) return;
          // invoke 自体の reject (バックエンドで Err を返した場合) はチャンネルの "error"
          // でも通知されるので、設定が無い等の重複は無視。
          const msg = typeof e === "string" ? e : (e as Error)?.message ?? String(e);
          if (!error) setError(msg);
          setStatus("error");
        });
      } catch (e: any) {
        if (cancelled) return;
        setError(typeof e === "string" ? e : (e?.message ?? String(e)));
        setStatus("error");
      }
    })();

    return () => { cancelled = true; };
  }, [entry.id, generationKey]);

  const handleSave = async () => {
    if (!text.trim() || !model || saving) return;
    setSaving(true);
    setSaveError(null);
    try {
      // entries.summary にも保存（履歴・モデル名・日時を保持）
      await invoke("save_entry_summary", { id: entry.id, summary: text, model });
      // ノートにも追記してユーザーが見える形で残す
      const stamp = new Date().toLocaleString();
      const header = `**要約** (${model} · ${stamp})`;
      const block = `${header}\n\n${text.trim()}`;
      const newNotes = entry.notes && entry.notes.trim().length > 0
        ? `${entry.notes}\n\n---\n\n${block}`
        : block;
      // 保存失敗は握りつぶさずユーザーに見せる（CR-034）。成功時のみ親が閉じる。
      await onSavedToNotes(newNotes);
    } catch (e) {
      const msg = typeof e === "string" ? e : (e as Error)?.message ?? String(e);
      setSaveError(msg);
    } finally {
      setSaving(false);
    }
  };

  // 生成中の再実行を禁止する（重複した有料リクエストの並走を防ぐ・CR-034）。
  const handleRegenerate = () => {
    if (status === "loading" || status === "streaming") return;
    setGenerationKey(k => k + 1);
  };

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
          width: 640, maxWidth: "92vw", maxHeight: "82vh",
          background: "var(--surface)",
          border: "1px solid var(--border-strong)",
          borderRadius: 10,
          boxShadow: "0 12px 32px rgba(0,0,0,0.18)",
          display: "flex", flexDirection: "column", overflow: "hidden",
        }}
      >
        <div style={{
          padding: "14px 18px",
          borderBottom: "1px solid var(--border)",
          display: "flex", alignItems: "center", gap: 10,
        }}>
          <Icon name="sparkle" size={14} color="var(--accent-strong)" />
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text)" }}>
              {t("summary.title")}
            </div>
            <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>
              {source === "fulltext" ? t("summary.subtitleFulltext") : t("summary.subtitleAbstract")}
              {status === "streaming" || status === "done" ? (
                <> · {t("summary.modelLabel", { model })}</>
              ) : null}
            </div>
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

        <div style={{ flex: 1, overflow: "auto", padding: "16px 20px", minHeight: 140 }}>
          {status === "loading" && (
            <div style={{ color: "var(--text-mute)", fontSize: 12 }}>{t("summary.generating")}</div>
          )}
          {status === "no_key" && (
            <div>
              <div style={{ fontSize: 13, fontWeight: 600, color: "var(--text)", marginBottom: 6 }}>
                {t("summary.noApiKey")}
              </div>
              <div style={{ fontSize: 12, color: "var(--text-mute)", marginBottom: 12 }}>
                {t("summary.noApiKeyHelp")}
              </div>
              <button
                onClick={onOpenSettings}
                style={{
                  padding: "6px 12px", borderRadius: 5,
                  border: "1px solid var(--border-strong)",
                  background: "var(--accent-strong)", color: "white",
                  fontSize: 12, fontWeight: 500, cursor: "pointer",
                }}
              >{t("summary.openSettings")}</button>
            </div>
          )}
          {(status === "streaming" || status === "done") && (
            <MathMarkdown value={text} />
          )}
          {status === "streaming" && (
            <span style={{
              display: "inline-block", width: 7, height: 14,
              background: "var(--accent-strong)", marginLeft: 2,
              animation: "pulse 1s ease-in-out infinite",
              verticalAlign: "text-bottom",
            }} />
          )}
          {status === "error" && (
            <div>
              <div style={{ fontSize: 13, fontWeight: 600, color: "var(--danger-strong)", marginBottom: 6 }}>
                {t("summary.errorTitle")}
              </div>
              <div style={{ fontSize: 12, color: "var(--text-mute)", whiteSpace: "pre-wrap" }}>{error}</div>
            </div>
          )}
        </div>

        <div style={{
          padding: "12px 18px", borderTop: "1px solid var(--border)",
          display: "flex", justifyContent: "flex-end", gap: 6, alignItems: "center",
        }}>
          {saveError && (
            <span style={{ marginRight: "auto", fontSize: 11.5, color: "var(--danger-strong)" }}>
              {t("summary.saveError")}: {saveError}
            </span>
          )}
          <button
            onClick={onClose}
            style={{
              padding: "6px 12px", borderRadius: 5,
              border: "1px solid var(--border-strong)",
              background: "var(--surface)", color: "var(--text)",
              fontSize: 12, cursor: "pointer",
            }}
          >{t("summary.close")}</button>
          {(status === "done" || status === "error") && (
            <button
              onClick={handleRegenerate}
              style={{
                padding: "6px 12px", borderRadius: 5,
                border: "1px solid var(--border-strong)",
                background: "var(--surface)", color: "var(--text)",
                fontSize: 12, cursor: "pointer",
              }}
            >{t("summary.regenerate")}</button>
          )}
          {status === "done" && (
            <button
              onClick={handleSave}
              disabled={saving}
              style={{
                padding: "6px 12px", borderRadius: 5,
                border: "1px solid var(--border-strong)",
                background: "var(--accent-strong)", color: "white",
                fontSize: 12, fontWeight: 500, cursor: saving ? "default" : "pointer",
                opacity: saving ? 0.6 : 1,
              }}
            >{t("summary.save")}</button>
          )}
        </div>
      </div>
    </div>
  );
}
