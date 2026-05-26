// NewSessionDialog — プロバイダ/モデルと初期スコープを選んでセッション作成。
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { useChatStore } from "../../chat/store";
import { ChatIcon } from "./ChatIcon";
import { EntryPicker, ScopeModeButton } from "./EntryPicker";
import type { EntrySummary, LlmProvider, LlmSettings, ScopeMode } from "../../types";

interface NewSessionDialogProps {
  onClose: () => void;
}

export function NewSessionDialog({ onClose }: NewSessionDialogProps) {
  const { t } = useTranslation();
  const createSession = useChatStore((s) => s.createSession);
  const [provider, setProvider] = useState<LlmProvider>("anthropic");
  const [model, setModel] = useState("");
  const [scope, setScope] = useState<ScopeMode>("all");
  const [picked, setPicked] = useState<Map<number, string>>(() => new Map());

  // 既定のプロバイダ/モデルを設定から読む
  useEffect(() => {
    void (async () => {
      try {
        const s = await invoke<LlmSettings>("get_llm_settings");
        setProvider(s.provider);
        setModel(s.model);
      } catch {
        setModel("");
      }
    })();
  }, []);

  const toggle = (e: EntrySummary) => {
    setPicked((prev) => {
      const next = new Map(prev);
      if (next.has(e.id)) next.delete(e.id);
      else next.set(e.id, e.title);
      return next;
    });
  };

  const create = async () => {
    if (!model.trim()) return;
    const entryIds = scope === "entries" ? [...picked.keys()] : [];
    await createSession({ title: t("chat.newChat"), provider, model: model.trim(), scopeMode: scope, entryIds });
    onClose();
  };

  return (
    <div onClick={onClose} style={{ position: "absolute", inset: 0, zIndex: 40, background: "rgba(20,18,14,0.32)", backdropFilter: "blur(2px)", display: "flex", alignItems: "flex-start", justifyContent: "center", paddingTop: 90 }}>
      <div onClick={(e) => e.stopPropagation()} style={{ width: 520, maxHeight: "80%", overflow: "auto", background: "var(--surface)", borderRadius: 10, border: "1px solid var(--border-strong)", boxShadow: "0 24px 60px rgba(0,0,0,0.22)" }}>
        <div style={{ padding: "14px 18px 12px", borderBottom: "1px solid var(--border)", display: "flex", alignItems: "center", gap: 10 }}>
          <div style={{ width: 26, height: 26, borderRadius: 6, background: "color-mix(in oklch, var(--accent-strong) 14%, var(--surface))", display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
            <ChatIcon name="sparkle" size={14} color="var(--accent-strong)" />
          </div>
          <div>
            <div style={{ fontSize: 13.5, fontWeight: 600, color: "var(--text)" }}>{t("chat.newSessionTitle")}</div>
            <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>{t("chat.newSessionSub")}</div>
          </div>
        </div>

        <div style={{ padding: "16px 18px 8px" }}>
          <FieldLabel>{t("chat.providerModel")}</FieldLabel>
          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <Pill dotColor="oklch(0.62 0.14 35)" label="Anthropic" active={provider === "anthropic"} onClick={() => setProvider("anthropic")} />
            <Pill dotColor="oklch(0.55 0.13 165)" label="OpenAI" active={provider === "openai"} onClick={() => setProvider("openai")} />
            <input
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder={t("chat.model")}
              style={{ flex: 1, minWidth: 0, padding: "5px 10px", borderRadius: 5, border: "1px solid var(--border-strong)", background: "var(--surface)", fontSize: 12, fontFamily: "var(--mono)", color: "var(--text)" }}
            />
          </div>
        </div>

        <div style={{ padding: "10px 18px 14px" }}>
          <FieldLabel>{t("chat.initialScope")}</FieldLabel>
          <div style={{ display: "flex", gap: 8 }}>
            <ScopeModeButton label={t("chat.scopeModeAll")} sub={t("chat.scopeModeAllSub")} active={scope === "all"} onClick={() => setScope("all")} />
            <ScopeModeButton label={t("chat.scopeModeEntries")} sub={t("chat.selectedCount", { count: picked.size })} active={scope === "entries"} onClick={() => setScope("entries")} />
          </div>
          {scope === "entries" && (
            <div style={{ marginTop: 10, padding: "8px 10px", background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 6 }}>
              <EntryPicker selected={new Set(picked.keys())} onToggle={toggle} maxHeight={200} />
            </div>
          )}
        </div>

        <div style={{ padding: "10px 16px", borderTop: "1px solid var(--border)", background: "var(--surface-2)", display: "flex", justifyContent: "flex-end", gap: 8, position: "sticky", bottom: 0 }}>
          <button onClick={onClose} style={btnGhost}>{t("chat.cancel")}</button>
          <button onClick={() => void create()} disabled={!model.trim()} style={{ ...btnPrimary, opacity: model.trim() ? 1 : 0.5 }}>{t("chat.start")}</button>
        </div>
      </div>
    </div>
  );
}

function FieldLabel({ children }: { children: React.ReactNode }) {
  return <div style={{ fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)", textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6 }}>{children}</div>;
}

function Pill({ dotColor, label, active, onClick }: { dotColor: string; label: string; active: boolean; onClick: () => void }) {
  return (
    <button onClick={onClick} style={{ display: "inline-flex", alignItems: "center", gap: 6, padding: "5px 12px", borderRadius: 999, cursor: "pointer", border: "1px solid " + (active ? "var(--accent-strong)" : "var(--border-strong)"), background: active ? "var(--accent-soft)" : "var(--surface)", color: active ? "var(--accent-strong)" : "var(--text)", fontSize: 12, fontWeight: 500 }}>
      <span style={{ width: 7, height: 7, borderRadius: "50%", background: dotColor }} />
      {label}
    </button>
  );
}

const btnGhost: React.CSSProperties = { padding: "6px 14px", borderRadius: 5, border: "1px solid var(--border-strong)", background: "var(--surface)", color: "var(--text)", fontSize: 12, cursor: "pointer" };
const btnPrimary: React.CSSProperties = { padding: "6px 16px", borderRadius: 5, border: "none", background: "var(--accent-strong)", color: "white", fontSize: 12, fontWeight: 600, cursor: "pointer" };
