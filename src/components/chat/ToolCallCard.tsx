// ToolCallCard — 5 系統 (read/write/approve/delete/mcp) × 状態 (running/needs_approval/done/rejected)。
// design handoff の ToolCallCard を実データ (UiToolCall) 向けに実装。
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChatIcon, type ChatIconName } from "./ChatIcon";
import type { UiToolCall } from "../../types";

export type ToolKind = "read" | "write" | "approve" | "delete" | "mcp";

export const KIND_META: Record<ToolKind, { fg: string; bg: string; bd: string; label: string; glyph: ChatIconName }> = {
  read: { fg: "var(--tc-read-fg)", bg: "var(--tc-read-bg)", bd: "var(--tc-read-bd)", label: "read", glyph: "search" },
  write: { fg: "var(--tc-write-fg)", bg: "var(--tc-write-bg)", bd: "var(--tc-write-bd)", label: "write", glyph: "pencil" },
  approve: { fg: "var(--tc-approve-fg)", bg: "var(--tc-approve-bg)", bd: "var(--tc-approve-bd)", label: "approval required", glyph: "warn" },
  delete: { fg: "var(--tc-delete-fg)", bg: "var(--tc-delete-bg)", bd: "var(--tc-delete-bd)", label: "destructive", glyph: "trash" },
  mcp: { fg: "var(--tc-mcp-fg)", bg: "var(--tc-mcp-bg)", bd: "var(--tc-mcp-bd)", label: "mcp", glyph: "plug" },
};

/** tool_name から表示カテゴリを導出（承認ポリシーと同じ分類）。 */
export function toolKind(name: string): ToolKind {
  if (name.startsWith("mcp_")) return "mcp";
  if (name.startsWith("delete_")) return "delete";
  if (name === "create_entry" || name === "update_entry") return "approve";
  if (name === "fulltext_search" || name === "get_entry" || name.startsWith("list_")) return "read";
  return "write";
}

function mcpServer(name: string): string | null {
  if (!name.startsWith("mcp_")) return null;
  const parts = name.split("_");
  return parts.length >= 3 ? parts[1] : null;
}

interface ToolCallCardProps {
  tc: UiToolCall;
  onApprove: (callId: string, approved: boolean) => void;
}

export function ToolCallCard({ tc, onApprove }: ToolCallCardProps) {
  const { t } = useTranslation();
  const kind = toolKind(tc.tool_name);
  const k = KIND_META[kind];
  const pending = tc.state === "needs_approval";
  const running = tc.state === "running";
  const rejected = tc.state === "rejected";
  const louder = pending || kind === "delete";
  const [open, setOpen] = useState(pending);

  const parsedArgs = tryParse(tc.args_preview);

  return (
    <div
      style={{
        borderRadius: 7, overflow: "hidden",
        border: "1px solid " + (louder ? k.bd : "var(--border)"),
        background: louder ? k.bg : "var(--surface)",
        opacity: rejected ? 0.7 : 1,
        animation: pending ? "pulseApprove 2.4s ease-in-out infinite" : undefined,
        transition: "background 120ms ease, opacity 120ms ease",
      }}
    >
      {/* Header */}
      <button
        onClick={() => { if (!pending) setOpen((o) => !o); }}
        style={{ width: "100%", border: "none", background: "transparent", padding: "8px 10px", display: "flex", alignItems: "center", gap: 9, cursor: pending ? "default" : "pointer", textAlign: "left", color: "var(--text)" }}
      >
        <span style={{ display: "inline-flex", alignItems: "center", justifyContent: "center", width: 22, height: 22, borderRadius: 5, flexShrink: 0, color: k.fg, background: louder ? "var(--surface)" : "color-mix(in oklch, " + k.fg + " 12%, transparent)", border: louder ? "1px solid " + k.bd : "none" }}>
          <ChatIcon name={k.glyph} size={12} color={k.fg} strokeWidth={1.6} />
        </span>

        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 1 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11.5, fontFamily: "var(--mono)", fontWeight: 500, minWidth: 0 }}>
            <span style={{ color: k.fg, fontWeight: 600 }}>{tc.tool_name}</span>
            <span style={{ color: "var(--text-faint)" }}>(</span>
            <span style={{ color: "var(--text-mute)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 0, flex: 1 }}>{tc.args_preview}</span>
            <span style={{ color: "var(--text-faint)" }}>)</span>
          </div>
          <div style={{ fontSize: 11, color: "var(--text-mute)", display: "flex", alignItems: "center", gap: 6 }}>
            {running && <Spinner color={k.fg} />}
            {running ? t("chat.toolRunning")
              : rejected ? <span style={{ color: "var(--text-faint)" }}>{t("chat.toolRejected")}</span>
              : pending ? <span style={{ color: k.fg, fontWeight: 500 }}>{t("chat.toolPending")}</span>
              : <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{firstLine(tc.result_summary)}</span>}
          </div>
        </div>

        <KindBadge kind={kind} server={mcpServer(tc.tool_name)} label={k.label} fg={k.fg} />
        {!pending && !running && (
          <span style={{ display: "inline-flex", color: "var(--text-faint)", transform: open ? "rotate(90deg)" : "rotate(0deg)", transition: "transform 120ms ease", marginLeft: 2 }}>
            <ChatIcon name="chevronRight" size={11} color="var(--text-faint)" />
          </span>
        )}
      </button>

      {/* Body */}
      {open && !running && (
        <div style={{ borderTop: "1px solid " + (louder ? k.bd : "var(--border)"), padding: "10px 12px 12px", background: louder ? "color-mix(in oklch, var(--surface) 60%, " + k.bg + ")" : "var(--surface-2)", display: "flex", flexDirection: "column", gap: 9 }}>
          <Section label={t("chat.argumentsLabel")}>
            {parsedArgs !== undefined ? <JsonPreview value={parsedArgs} /> : <CodeBlock text={tc.args_preview} />}
          </Section>
          {tc.result_summary && tc.state !== "rejected" && (
            <Section label={t("chat.resultLabel")}>
              <CodeBlock text={tc.result_summary} />
            </Section>
          )}

          {pending && (
            <div style={{ marginTop: 2, display: "flex", alignItems: "center", gap: 8, padding: "8px 10px", borderRadius: 6, background: "color-mix(in oklch, " + k.fg + " 12%, var(--surface))", border: "1px solid " + k.bd }}>
              <ChatIcon name="warn" size={13} color={k.fg} />
              <span style={{ flex: 1, fontSize: 11.5, color: "var(--text)", lineHeight: 1.4 }}>{t("chat.approvalBarText")}</span>
              <button onClick={() => onApprove(tc.call_id, false)} style={{ padding: "5px 11px", borderRadius: 5, border: "1px solid var(--border-strong)", background: "var(--surface)", color: "var(--text)", fontSize: 11.5, fontWeight: 500, cursor: "pointer" }}>{t("chat.reject")}</button>
              <button onClick={() => onApprove(tc.call_id, true)} style={{ display: "inline-flex", alignItems: "center", gap: 5, padding: "5px 13px", borderRadius: 5, border: "none", background: k.fg, color: "white", fontSize: 11.5, fontWeight: 600, cursor: "pointer", boxShadow: "0 1px 0 oklch(0 0 0 / 0.1)" }}>
                <ChatIcon name="check" size={11} color="white" />
                {t("chat.approve")}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function KindBadge({ kind, server, label, fg }: { kind: ToolKind; server: string | null; label: string; fg: string }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 9.5, fontWeight: 600, letterSpacing: "0.06em", textTransform: "uppercase", color: fg, padding: "1px 6px", borderRadius: 3, background: "color-mix(in oklch, " + fg + " 12%, transparent)", border: "1px solid color-mix(in oklch, " + fg + " 25%, transparent)", fontFamily: "var(--mono)", flexShrink: 0 }}>
      {kind === "mcp" ? (<>{server && <span style={{ opacity: 0.7 }}>{server}</span>}<span>mcp</span></>) : label}
    </span>
  );
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <div style={{ fontSize: 9.5, fontWeight: 600, letterSpacing: "0.08em", color: "var(--text-faint)", textTransform: "uppercase", marginBottom: 4 }}>{label}</div>
      {children}
    </div>
  );
}

function CodeBlock({ text }: { text: string }) {
  return (
    <div style={{ fontFamily: "var(--mono)", fontSize: 11, lineHeight: 1.55, color: "var(--text)", padding: "8px 10px", background: "var(--surface)", border: "1px solid var(--border)", borderRadius: 5, overflowX: "auto", whiteSpace: "pre-wrap", wordBreak: "break-word", maxHeight: 240, overflowY: "auto" }}>{text}</div>
  );
}

function Spinner({ color }: { color: string }) {
  return <span style={{ display: "inline-block", width: 11, height: 11, border: "1.4px solid color-mix(in oklch, " + color + " 30%, transparent)", borderTopColor: color, borderRadius: "50%", animation: "spin 0.7s linear infinite" }} />;
}

function firstLine(s?: string): string {
  if (!s) return "";
  const nl = s.indexOf("\n");
  return nl === -1 ? s : s.slice(0, nl);
}

function tryParse(s: string): unknown {
  const trimmed = s.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) return undefined;
  try {
    return JSON.parse(trimmed);
  } catch {
    return undefined;
  }
}

// 引数 JSON を控えめに色付けして表示。
function JsonPreview({ value }: { value: unknown }) {
  return (
    <div style={{ fontFamily: "var(--mono)", fontSize: 11, lineHeight: 1.55, color: "var(--text)", padding: "8px 10px", background: "var(--surface)", border: "1px solid var(--border)", borderRadius: 5, overflowX: "auto", whiteSpace: "pre-wrap" }}>
      {renderJson(value)}
    </div>
  );
}

function renderJson(v: unknown): React.ReactNode {
  if (v === null) return <span style={{ color: "var(--text-faint)" }}>null</span>;
  if (typeof v === "string") return <span style={{ color: "oklch(0.42 0.10 145)" }}>"{v}"</span>;
  if (typeof v === "number" || typeof v === "boolean") return <span style={{ color: "oklch(0.5 0.13 270)" }}>{String(v)}</span>;
  if (Array.isArray(v)) {
    return (
      <>
        <span>[</span>
        {v.map((x, i) => (
          <span key={i}>{renderJson(x)}{i < v.length - 1 ? ", " : ""}</span>
        ))}
        <span>]</span>
      </>
    );
  }
  if (typeof v === "object") {
    const entries = Object.entries(v as Record<string, unknown>);
    return (
      <>
        <span>{"{"}</span>
        <div style={{ paddingLeft: 12 }}>
          {entries.map(([key, val], i) => (
            <div key={key}>
              <span style={{ color: "oklch(0.48 0.04 30)" }}>{key}</span>
              <span style={{ color: "var(--text-faint)" }}>: </span>
              {renderJson(val)}
              {i < entries.length - 1 ? <span>,</span> : null}
            </div>
          ))}
        </div>
        <span>{"}"}</span>
      </>
    );
  }
  return <span>{String(v)}</span>;
}
