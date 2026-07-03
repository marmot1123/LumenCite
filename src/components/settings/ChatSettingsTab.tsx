// 設定モーダルの「Chat」タブ: MCP サーバー管理 + ツール自動承認ホワイトリスト。
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Icon } from "../icons";
import type { ClipperStatusInfo, McpServerInfo, McpServerStatus, McpServerStatusInfo } from "../../types";

// ホワイトリストで上書き可能なツールと既定の自動承認可否（backend approval.rs と一致）。
const OVERRIDABLE_TOOLS: { name: string; defaultAuto: boolean }[] = [
  { name: "add_tag", defaultAuto: true },
  { name: "update_notes", defaultAuto: true },
  { name: "add_to_collection", defaultAuto: true },
  { name: "create_entry", defaultAuto: false },
  { name: "update_entry", defaultAuto: false },
];

const WHITELIST_KEY = "chat.tool_whitelist";

export function ChatSettingsTab() {
  const { t } = useTranslation();
  return (
    <>
      <div style={{ fontSize: 12, color: "var(--text-mute)", marginBottom: 18, lineHeight: 1.55 }}>
        {t("settings.chat.description")}
      </div>
      <McpServers />
      <McpServerPublic />
      <ClipperSettings />
      <ToolWhitelist />
    </>
  );
}

/** `KEY=VALUE` の行群を環境変数マップに変換する。空行・`#` コメント・`=` 無しの行は無視。 */
function parseEnv(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    const key = trimmed.slice(0, eq).trim();
    if (key) out[key] = trimmed.slice(eq + 1).trim();
  }
  return out;
}

/** MCP サーバーの起動状態を色付きの小バッジで表示する。 */
function McpStatusBadge({ status }: { status: McpServerStatus | null }) {
  const { t } = useTranslation();
  const base: React.CSSProperties = {
    fontSize: 10, fontWeight: 600, padding: "1px 6px", borderRadius: 999,
    whiteSpace: "nowrap", flexShrink: 0,
  };
  if (!status) {
    return <span style={{ ...base, color: "var(--text-faint)", background: "var(--surface)" }}>{t("settings.chat.mcpStatusUnknown")}</span>;
  }
  if (status.state === "running") {
    return <span style={{ ...base, color: "#15803d", background: "rgba(34,197,94,0.14)" }}>● {t("settings.chat.mcpStatusRunning", { count: status.tool_count })}</span>;
  }
  return <span style={{ ...base, color: "var(--danger-strong)", background: "var(--danger-bg)" }}>● {t("settings.chat.mcpStatusFailed")}</span>;
}

function McpServers() {
  const { t } = useTranslation();
  const [servers, setServers] = useState<McpServerInfo[]>([]);
  const [id, setId] = useState("");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [env, setEnv] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = () => {
    invoke<McpServerInfo[]>("list_mcp_servers").then(setServers).catch(() => setServers([]));
  };
  useEffect(reload, []);

  // 既存サーバーの再起動（env 修正後など）。add_mcp_server は同 config を保存し直して
  // start を走らせる。成否に関わらず backend が status を更新するので reload で反映する。
  const retry = async (s: McpServerInfo) => {
    setBusy(true);
    try {
      await invoke("add_mcp_server", {
        config: { id: s.id, command: s.command, args: s.args, env: s.env },
      });
    } catch { /* status は backend 側で更新済み */ }
    finally { setBusy(false); reload(); }
  };

  const add = async () => {
    if (!id.trim() || !command.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("add_mcp_server", {
        config: {
          id: id.trim(),
          command: command.trim(),
          args: args.trim() ? args.trim().split(/\s+/) : [],
          env: parseEnv(env),
        },
      });
      setId(""); setCommand(""); setArgs(""); setEnv("");
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
      // 成否に関わらず一覧を更新。起動失敗でも config は保存済みなので、
      // 失敗サーバーが赤バッジ + 再起動ボタン付きで一覧に現れる。
      reload();
    }
  };

  const remove = async (sid: string) => {
    await invoke("remove_mcp_server", { id: sid }).catch(console.error);
    reload();
  };

  return (
    <Section title={t("settings.chat.mcpTitle")} description={t("settings.chat.mcpDesc")}>
      {servers.length > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 10 }}>
          {servers.map((s) => (
            <div key={s.id} style={{ display: "flex", alignItems: "center", gap: 8, padding: "7px 10px", borderRadius: 6, border: "1px solid var(--border)", background: "var(--surface-2)" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                  <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text)" }}>{s.id}</span>
                  <McpStatusBadge status={s.status} />
                </div>
                <div style={{ fontSize: 10.5, color: "var(--text-faint)", fontFamily: "var(--mono)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                  {s.command} {s.args.join(" ")}
                </div>
                {Object.keys(s.env ?? {}).length > 0 && (
                  <div style={{ fontSize: 10, color: "var(--text-faint)", fontFamily: "var(--mono)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                    env: {Object.keys(s.env).join(", ")}
                  </div>
                )}
                {s.status?.state === "failed" && (
                  <div title={s.status.error} style={{ fontSize: 10, color: "var(--danger-strong)", marginTop: 2, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                    {s.status.error}
                  </div>
                )}
              </div>
              {s.status?.state === "failed" && (
                <button onClick={() => void retry(s)} disabled={busy} title={t("settings.chat.mcpRetry")} style={{ ...iconBtn, opacity: busy ? 0.5 : 1 }}>
                  <Icon name="sync" size={13} color="var(--text-mute)" />
                </button>
              )}
              <button onClick={() => void remove(s.id)} title={t("settings.chat.mcpRemove")} style={iconBtn}>
                <Icon name="trash" size={13} color="var(--danger-strong)" />
              </button>
            </div>
          ))}
        </div>
      )}
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        <div style={{ display: "flex", gap: 6 }}>
          <input value={id} onChange={(e) => setId(e.target.value)} placeholder={t("settings.chat.mcpId")} style={{ ...field, width: 120 }} />
          <input value={command} onChange={(e) => setCommand(e.target.value)} placeholder={t("settings.chat.mcpCommand")} style={{ ...field, flex: 1 }} />
        </div>
        <input value={args} onChange={(e) => setArgs(e.target.value)} placeholder={t("settings.chat.mcpArgs")} style={field} />
        <textarea
          value={env}
          onChange={(e) => setEnv(e.target.value)}
          placeholder={t("settings.chat.mcpEnv")}
          rows={2}
          spellCheck={false}
          style={{ ...field, resize: "vertical", lineHeight: 1.5 }}
        />
        <div style={{ fontSize: 10.5, color: "var(--text-faint)", lineHeight: 1.5 }}>{t("settings.chat.mcpEnvNote")}</div>
        {error && <div style={{ fontSize: 11.5, color: "var(--danger-strong)" }}>{error}</div>}
        <button onClick={() => void add()} disabled={busy || !id.trim() || !command.trim()} style={{ ...primaryBtn, opacity: busy || !id.trim() || !command.trim() ? 0.5 : 1 }}>
          <Icon name="plus" size={12} color="#fff" />
          {t("settings.chat.mcpAdd")}
        </button>
      </div>
    </Section>
  );
}

// LumenCite 自身を MCP サーバーとして公開する設定。
// 有効化するとアプリ内に localhost HTTP サーバーが立ち、Claude Code 等から接続できる。
// 既定は read-only。write 系ツールは別トグル（Phase 2）で明示的に許可する。
function McpServerPublic() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<McpServerStatusInfo | null>(null);
  const [snippet, setSnippet] = useState("");
  const [desktopSnippet, setDesktopSnippet] = useState("");
  const [codexSnippet, setCodexSnippet] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [copiedDesktop, setCopiedDesktop] = useState(false);
  const [copiedCodex, setCopiedCodex] = useState(false);

  const reload = () => {
    invoke<McpServerStatusInfo>("get_mcp_server_status").then(setStatus).catch(() => setStatus(null));
  };
  useEffect(reload, []);

  const enabled = status?.enabled ?? false;
  const running = status?.running ?? false;
  const writeEnabled = status?.write_enabled ?? false;
  const port = status?.port;

  // 起動中はクライアント設定スニペット（token 込み）を取得する。
  // Claude Code = リモート HTTP 直結コマンド / Claude Desktop = stdio shim 用 mcpServers JSON。
  useEffect(() => {
    if (enabled && running) {
      invoke<string>("get_mcp_server_config_snippet", { client: "claude_code" })
        .then(setSnippet)
        .catch(() => setSnippet(""));
      invoke<string>("get_mcp_server_config_snippet", { client: "claude_desktop" })
        .then(setDesktopSnippet)
        .catch(() => setDesktopSnippet(""));
      invoke<string>("get_mcp_server_config_snippet", { client: "codex" })
        .then(setCodexSnippet)
        .catch(() => setCodexSnippet(""));
    } else {
      setSnippet("");
      setDesktopSnippet("");
      setCodexSnippet("");
    }
  }, [enabled, running, port]);

  const toggle = async (next: boolean) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await invoke<McpServerStatusInfo>("set_mcp_server_enabled", { enabled: next }));
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      reload();
    } finally {
      setBusy(false);
    }
  };

  const regenerate = async () => {
    setBusy(true);
    setError(null);
    try {
      await invoke<string>("regenerate_mcp_server_token");
      if (enabled && running) {
        setSnippet(await invoke<string>("get_mcp_server_config_snippet", { client: "claude_code" }));
        setDesktopSnippet(await invoke<string>("get_mcp_server_config_snippet", { client: "claude_desktop" }));
        setCodexSnippet(await invoke<string>("get_mcp_server_config_snippet", { client: "codex" }));
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  };

  // write 系ツールの公開可否（Phase 2）。サーバー再起動は不要（backend がリクエスト毎に評価）。
  const toggleWrite = async (next: boolean) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await invoke<McpServerStatusInfo>("set_mcp_server_write_enabled", { enabled: next }));
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      reload();
    } finally {
      setBusy(false);
    }
  };

  const copyTo = async (text: string, setFlag: (v: boolean) => void) => {
    if (!text) return;
    try {
      await writeText(text);
      setFlag(true);
      setTimeout(() => setFlag(false), 1500);
    } catch { /* clipboard 失敗は無視 */ }
  };

  return (
    <Section title={t("settings.chat.mcpServerTitle")} description={t("settings.chat.mcpServerDesc")}>
      <label style={{ display: "flex", alignItems: "center", gap: 10, padding: "6px 8px", borderRadius: 6, cursor: busy ? "default" : "pointer" }}>
        <input type="checkbox" checked={enabled} disabled={busy} onChange={(e) => void toggle(e.target.checked)} />
        <span style={{ fontSize: 12.5, color: "var(--text)" }}>{t("settings.chat.mcpServerEnable")}</span>
        <span style={{ flex: 1 }} />
        {enabled && (running ? (
          <span style={{ ...badge, color: "#15803d", background: "rgba(34,197,94,0.14)" }}>
            ● {t("settings.chat.mcpServerRunning", { port })}
          </span>
        ) : (
          <span style={{ ...badge, color: "var(--danger-strong)", background: "var(--danger-bg)" }}>
            ● {t("settings.chat.mcpServerStopped")}
          </span>
        ))}
      </label>
      {error && <div style={{ fontSize: 11.5, color: "var(--danger-strong)", marginTop: 6 }}>{error}</div>}
      {enabled && (
        <div style={{ marginTop: 6 }}>
          <label style={{ display: "flex", alignItems: "center", gap: 10, padding: "6px 8px", borderRadius: 6, cursor: busy ? "default" : "pointer" }}>
            <input type="checkbox" checked={writeEnabled} disabled={busy} onChange={(e) => void toggleWrite(e.target.checked)} />
            <span style={{ fontSize: 12.5, color: "var(--text)" }}>{t("settings.chat.mcpServerWriteEnable")}</span>
          </label>
          <div style={{ fontSize: 10.5, color: "var(--text-faint)", lineHeight: 1.5, padding: "0 8px" }}>
            {t("settings.chat.mcpServerWriteWarn")}
          </div>
        </div>
      )}
      {enabled && running && (
        <div style={{ marginTop: 10, display: "flex", flexDirection: "column", gap: 6 }}>
          {/* Claude Code: リモート HTTP MCP に直結するコマンド */}
          <div style={{ fontSize: 11.5, fontWeight: 600, color: "var(--text)" }}>{t("settings.chat.mcpServerCodeLabel")}</div>
          <div style={{ fontSize: 11, color: "var(--text-mute)", lineHeight: 1.5 }}>{t("settings.chat.mcpServerSnippetNote")}</div>
          <textarea
            readOnly
            value={snippet}
            rows={3}
            spellCheck={false}
            onFocus={(e) => e.currentTarget.select()}
            style={{ ...field, resize: "vertical", lineHeight: 1.5, fontSize: 11 }}
          />
          <div style={{ display: "flex", gap: 6 }}>
            <button onClick={() => void copyTo(snippet, setCopied)} disabled={!snippet} style={{ ...primaryBtn, opacity: snippet ? 1 : 0.5 }}>
              {copied ? t("settings.chat.mcpServerCopied") : t("settings.chat.mcpServerCopy")}
            </button>
            <button onClick={() => void regenerate()} disabled={busy} title={t("settings.chat.mcpServerRegenNote")} style={{ ...iconBtn, width: "auto", padding: "0 10px", gap: 6, fontSize: 11.5, color: "var(--text-mute)" }}>
              <Icon name="sync" size={12} color="var(--text-mute)" />
              {t("settings.chat.mcpServerRegen")}
            </button>
          </div>

          {/* Claude Desktop: stdio のみ対応 → 本体を --mcp-stdio shim として起動する mcpServers JSON */}
          <div style={{ marginTop: 8, fontSize: 11.5, fontWeight: 600, color: "var(--text)" }}>{t("settings.chat.mcpServerDesktopLabel")}</div>
          <div style={{ fontSize: 11, color: "var(--text-mute)", lineHeight: 1.5 }}>{t("settings.chat.mcpServerDesktopNote")}</div>
          {/* command にこのアプリの絶対パスを埋め込むため、移動/translocation で壊れる旨を警告 */}
          <div style={{ fontSize: 10.5, color: "var(--text-faint)", lineHeight: 1.5 }}>⚠️ {t("settings.chat.mcpServerDesktopPathWarn")}</div>
          <textarea
            readOnly
            value={desktopSnippet}
            rows={10}
            spellCheck={false}
            onFocus={(e) => e.currentTarget.select()}
            style={{ ...field, resize: "vertical", lineHeight: 1.5, fontSize: 11, fontFamily: "var(--font-mono, monospace)" }}
          />
          <div style={{ display: "flex", gap: 6 }}>
            <button onClick={() => void copyTo(desktopSnippet, setCopiedDesktop)} disabled={!desktopSnippet} style={{ ...primaryBtn, opacity: desktopSnippet ? 1 : 0.5 }}>
              {copiedDesktop ? t("settings.chat.mcpServerCopied") : t("settings.chat.mcpServerDesktopCopy")}
            </button>
          </div>

          {/* Codex (OpenAI CLI): stdio のみ → 本体を --mcp-stdio shim として起動する config.toml の [mcp_servers.*] */}
          <div style={{ marginTop: 8, fontSize: 11.5, fontWeight: 600, color: "var(--text)" }}>{t("settings.chat.mcpServerCodexLabel")}</div>
          <div style={{ fontSize: 11, color: "var(--text-mute)", lineHeight: 1.5 }}>{t("settings.chat.mcpServerCodexNote")}</div>
          <div style={{ fontSize: 10.5, color: "var(--text-faint)", lineHeight: 1.5 }}>⚠️ {t("settings.chat.mcpServerDesktopPathWarn")}</div>
          <textarea
            readOnly
            value={codexSnippet}
            rows={4}
            spellCheck={false}
            onFocus={(e) => e.currentTarget.select()}
            style={{ ...field, resize: "vertical", lineHeight: 1.5, fontSize: 11, fontFamily: "var(--font-mono, monospace)" }}
          />
          <div style={{ display: "flex", gap: 6 }}>
            <button onClick={() => void copyTo(codexSnippet, setCopiedCodex)} disabled={!codexSnippet} style={{ ...primaryBtn, opacity: codexSnippet ? 1 : 0.5 }}>
              {copiedCodex ? t("settings.chat.mcpServerCopied") : t("settings.chat.mcpServerCodexCopy")}
            </button>
          </div>
        </div>
      )}
    </Section>
  );
}

/** Web クリッパー（Chrome 拡張）: 有効トグル + 拡張に貼る接続コード。 */
function ClipperSettings() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<ClipperStatusInfo | null>(null);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const reload = () => {
    invoke<ClipperStatusInfo>("get_clipper_status").then(setStatus).catch(() => setStatus(null));
  };
  useEffect(reload, []);

  const enabled = status?.enabled ?? false;
  const running = status?.server_running ?? false;
  const port = status?.port;

  useEffect(() => {
    if (enabled && running) {
      invoke<string>("get_clipper_connect_code").then(setCode).catch(() => setCode(""));
    } else {
      setCode("");
    }
  }, [enabled, running, port]);

  const toggle = async (next: boolean) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await invoke<ClipperStatusInfo>("set_clipper_enabled", { enabled: next }));
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      reload();
    } finally {
      setBusy(false);
    }
  };

  const copy = async () => {
    if (!code) return;
    try {
      await writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch { /* clipboard 失敗は無視 */ }
  };

  return (
    <Section title={t("settings.chat.clipperTitle")} description={t("settings.chat.clipperDesc")}>
      <label style={{ display: "flex", alignItems: "center", gap: 10, padding: "6px 8px", borderRadius: 6, cursor: busy ? "default" : "pointer" }}>
        <input type="checkbox" checked={enabled} disabled={busy} onChange={(e) => void toggle(e.target.checked)} />
        <span style={{ fontSize: 12.5, color: "var(--text)" }}>{t("settings.chat.clipperEnable")}</span>
        <span style={{ flex: 1 }} />
        {enabled && (running ? (
          <span style={{ ...badge, color: "#15803d", background: "rgba(34,197,94,0.14)" }}>
            ● {t("settings.chat.clipperRunning", { port })}
          </span>
        ) : (
          <span style={{ ...badge, color: "var(--danger-strong)", background: "var(--danger-bg)" }}>
            ● {t("settings.chat.clipperStopped")}
          </span>
        ))}
      </label>
      {error && <div style={{ fontSize: 11.5, color: "var(--danger-strong)", marginTop: 6 }}>{error}</div>}
      {enabled && running && (
        <div style={{ marginTop: 10, display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ fontSize: 11.5, fontWeight: 600, color: "var(--text)" }}>{t("settings.chat.clipperCodeLabel")}</div>
          <div style={{ fontSize: 11, color: "var(--text-mute)", lineHeight: 1.5 }}>{t("settings.chat.clipperCodeNote")}</div>
          {/* トークンは MCP サーバーと共有: 「トークン再生成」で拡張のペアリングも無効になる */}
          <div style={{ fontSize: 10.5, color: "var(--text-faint)", lineHeight: 1.5 }}>⚠️ {t("settings.chat.clipperRegenWarn")}</div>
          <textarea
            readOnly
            value={code}
            rows={2}
            spellCheck={false}
            onFocus={(e) => e.currentTarget.select()}
            style={{ ...field, resize: "vertical", lineHeight: 1.5, fontSize: 11, fontFamily: "var(--font-mono, monospace)" }}
          />
          <div style={{ display: "flex", gap: 6 }}>
            <button onClick={() => void copy()} disabled={!code} style={{ ...primaryBtn, opacity: code ? 1 : 0.5 }}>
              {copied ? t("settings.chat.mcpServerCopied") : t("settings.chat.clipperCopy")}
            </button>
          </div>
        </div>
      )}
    </Section>
  );
}

function ToolWhitelist() {
  const { t } = useTranslation();
  const [overrides, setOverrides] = useState<Record<string, boolean>>({});

  useEffect(() => {
    invoke<string | null>("get_setting", { key: WHITELIST_KEY })
      .then((json) => {
        if (!json) return;
        try { setOverrides(JSON.parse(json)); } catch { /* ignore */ }
      })
      .catch(() => {});
  }, []);

  const setAuto = (tool: string, auto: boolean) => {
    const next = { ...overrides, [tool]: auto };
    setOverrides(next);
    void invoke("set_setting", { key: WHITELIST_KEY, value: JSON.stringify(next) }).catch(console.error);
  };

  return (
    <Section title={t("settings.chat.whitelistTitle")} description={t("settings.chat.whitelistDesc")}>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {OVERRIDABLE_TOOLS.map(({ name, defaultAuto }) => {
          const auto = overrides[name] ?? defaultAuto;
          return (
            <label key={name} style={{ display: "flex", alignItems: "center", gap: 10, padding: "6px 8px", borderRadius: 6, cursor: "pointer" }}>
              <input type="checkbox" checked={auto} onChange={(e) => setAuto(name, e.target.checked)} />
              <span style={{ fontFamily: "var(--mono)", fontSize: 12, color: "var(--text)" }}>{name}</span>
              <span style={{ flex: 1 }} />
              <span style={{ fontSize: 10.5, color: "var(--text-faint)" }}>
                {auto ? t("settings.chat.auto") : t("settings.chat.confirm")}
              </span>
            </label>
          );
        })}
      </div>
      <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 8, lineHeight: 1.5 }}>
        {t("settings.chat.whitelistNote")}
      </div>
    </Section>
  );
}

function Section({ title, description, children }: { title: string; description?: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: 22 }}>
      <div style={{ fontSize: 13, fontWeight: 600, color: "var(--text)", marginBottom: description ? 2 : 8 }}>{title}</div>
      {description && <div style={{ fontSize: 11.5, color: "var(--text-mute)", marginBottom: 10, lineHeight: 1.5 }}>{description}</div>}
      {children}
    </div>
  );
}

const field: React.CSSProperties = {
  padding: "7px 10px", borderRadius: 6, border: "1px solid var(--border-strong)",
  background: "var(--surface)", color: "var(--text)", fontSize: 12.5, fontFamily: "var(--mono)", minWidth: 0,
};
const primaryBtn: React.CSSProperties = {
  display: "inline-flex", alignItems: "center", justifyContent: "center", gap: 6,
  padding: "8px 12px", borderRadius: 6, border: "none", background: "var(--accent-strong)",
  color: "#fff", fontSize: 12.5, fontWeight: 600, cursor: "pointer", alignSelf: "flex-start",
};
const iconBtn: React.CSSProperties = {
  width: 28, height: 28, padding: 0, border: "1px solid var(--border)", borderRadius: 6,
  background: "var(--surface)", cursor: "pointer", display: "inline-flex", alignItems: "center", justifyContent: "center", flexShrink: 0,
};
const badge: React.CSSProperties = {
  fontSize: 10, fontWeight: 600, padding: "1px 6px", borderRadius: 999, whiteSpace: "nowrap", flexShrink: 0,
};
