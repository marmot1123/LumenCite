import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";

interface BibtexSyncSheetProps {
  onClose: () => void;
  initialPath: string | null;
  lastSynced: string | null; // epoch seconds (文字列) を想定
  lastError: string | null;
  onPathChanged: (path: string | null) => void;
}

function formatEpochSecs(s: string | null): string | null {
  if (!s) return null;
  const n = parseInt(s, 10);
  if (!Number.isFinite(n)) return null;
  return new Date(n * 1000).toLocaleString();
}

export function BibtexSyncSheet({ onClose, initialPath, lastSynced, lastError, onPathChanged }: BibtexSyncSheetProps) {
  const { t } = useTranslation();
  const [path, setPath] = useState<string | null>(initialPath);
  const [busy, setBusy] = useState(false);
  const [excludeAbstractNote, setExcludeAbstractNote] = useState(false);

  useEffect(() => setPath(initialPath), [initialPath]);

  useEffect(() => {
    invoke<boolean>("get_bibtex_exclude_abstract_note")
      .then(setExcludeAbstractNote)
      .catch((e) => console.error(e));
  }, []);

  const toggleExcludeAbstractNote = async (next: boolean) => {
    setExcludeAbstractNote(next); // 楽観更新
    try {
      await invoke("set_bibtex_exclude_abstract_note", { exclude: next });
    } catch (e) {
      console.error(e);
      setExcludeAbstractNote(!next); // 失敗したら戻す
    }
  };

  const pick = async () => {
    setBusy(true);
    try {
      const picked = await invoke<string | null>("pick_bibtex_sync_path", {
        defaultName: path ? path.split("/").pop() : "references.bib",
      });
      if (picked) {
        await invoke("set_bibtex_sync_path", { path: picked });
        setPath(picked);
        onPathChanged(picked);
      }
    } catch (e) { console.error(e); }
    finally { setBusy(false); }
  };

  const clear = async () => {
    setBusy(true);
    try {
      await invoke("clear_bibtex_sync_path");
      setPath(null);
      onPathChanged(null);
    } catch (e) { console.error(e); }
    finally { setBusy(false); }
  };

  const syncNow = async () => {
    setBusy(true);
    try { await invoke("sync_bibtex_now"); }
    catch (e) { console.error(e); }
    finally { setBusy(false); }
  };

  const lastSyncedHuman = formatEpochSecs(lastSynced);

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
          width: 480, maxWidth: "90vw",
          background: "var(--surface)",
          border: "1px solid var(--border-strong)",
          borderRadius: 10,
          boxShadow: "0 12px 32px rgba(0,0,0,0.18)",
          padding: "20px 22px 18px",
        }}
      >
        <div style={{ fontSize: 15, fontWeight: 600, color: "var(--text)", marginBottom: 4 }}>
          {t("bibtexSyncSheet.title")}
        </div>
        <div style={{ fontSize: 12, color: "var(--text-mute)", lineHeight: 1.55, marginBottom: 16 }}>
          {t("bibtexSyncSheet.description")}
        </div>

        <div style={{
          fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
          textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 5,
        }}>{t("bibtexSyncSheet.destLabel")}</div>
        <div style={{
          padding: "8px 10px", borderRadius: 6,
          border: "1px solid var(--border)",
          background: "var(--surface-2)",
          fontSize: 12, color: path ? "var(--text)" : "var(--text-faint)",
          fontFamily: "var(--mono)",
          wordBreak: "break-all", marginBottom: 12, minHeight: 18,
        }}>
          {path ?? t("bibtexSyncSheet.notSet")}
        </div>

        <div style={{ display: "flex", gap: 6, marginBottom: 16, flexWrap: "wrap" }}>
          <button
            onClick={pick}
            disabled={busy}
            style={{
              padding: "5px 10px", borderRadius: 5,
              border: "1px solid var(--border-strong)",
              background: "var(--accent-strong)", color: "white",
              fontSize: 12, fontWeight: 500, cursor: busy ? "default" : "pointer",
            }}
          >{t("bibtexSyncSheet.pickFile")}</button>
          <button
            onClick={syncNow}
            disabled={busy || !path}
            style={{
              padding: "5px 10px", borderRadius: 5,
              border: "1px solid var(--border-strong)",
              background: "var(--surface)", color: !path ? "var(--text-faint)" : "var(--text)",
              fontSize: 12, cursor: (busy || !path) ? "default" : "pointer",
            }}
          >{t("bibtexSyncSheet.syncNow")}</button>
          {path && (
            <button
              onClick={clear}
              disabled={busy}
              style={{
                padding: "5px 10px", borderRadius: 5,
                border: "none", background: "transparent",
                color: "var(--text-faint)",
                fontSize: 12, cursor: busy ? "default" : "pointer",
                marginLeft: "auto",
              }}
            >{t("bibtexSyncSheet.clearSync")}</button>
          )}
        </div>

        <div style={{
          fontSize: 11.5, color: "var(--text-mute)", lineHeight: 1.5,
          padding: "8px 10px", borderRadius: 6, background: "var(--surface-2)",
          marginBottom: 12,
        }}>
          {lastError ? (
            <span style={{ color: "var(--danger-strong)" }}>{t("bibtexSyncSheet.syncError", { error: lastError })}</span>
          ) : lastSyncedHuman ? (
            <span>{t("bibtexSyncSheet.lastSynced", { time: lastSyncedHuman })}</span>
          ) : path ? (
            <span style={{ color: "var(--text-faint)" }}>{t("bibtexSyncSheet.neverSynced")}</span>
          ) : (
            <span style={{ color: "var(--text-faint)" }}>{t("bibtexSyncSheet.disabled")}</span>
          )}
        </div>

        <div style={{
          fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
          textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6,
        }}>{t("bibtexSyncSheet.optionsLabel")}</div>
        <label style={{
          display: "flex", alignItems: "flex-start", gap: 8,
          cursor: "pointer", marginBottom: 16,
        }}>
          <input
            type="checkbox"
            checked={excludeAbstractNote}
            onChange={(e) => toggleExcludeAbstractNote(e.target.checked)}
            style={{ marginTop: 2, cursor: "pointer" }}
          />
          <span>
            <span style={{ fontSize: 12.5, color: "var(--text)" }}>
              {t("bibtexSyncSheet.excludeAbstractNote")}
            </span>
            <span style={{ display: "block", fontSize: 11, color: "var(--text-mute)", lineHeight: 1.5, marginTop: 2 }}>
              {t("bibtexSyncSheet.excludeAbstractNoteHint")}
            </span>
          </span>
        </label>

        <div style={{ display: "flex", justifyContent: "flex-end" }}>
          <button
            onClick={onClose}
            style={{
              padding: "5px 12px", borderRadius: 5,
              border: "1px solid var(--border-strong)",
              background: "var(--surface)", color: "var(--text)",
              fontSize: 12, cursor: "pointer",
            }}
          >{t("common.close")}</button>
        </div>
      </div>
    </div>
  );
}
