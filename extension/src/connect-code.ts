// LumenCite 設定画面が発行する接続コードのパース。
// 形式: `lc1.` + base64url(JSON `{"v":1,"port":<u16>,"token":"<hex>"}`)（パディングなし）。

import type { ClipperConfig } from "./types.js";

export function parseConnectCode(code: string): ClipperConfig | null {
  const trimmed = code.trim();
  if (!trimmed.startsWith("lc1.")) return null;

  const b64url = trimmed.slice("lc1.".length);
  if (!b64url) return null;

  let json: string;
  try {
    // base64url → base64（atob はパディング無しも受けるが、+/ へ戻す必要がある）
    const b64 = b64url.replace(/-/g, "+").replace(/_/g, "/");
    json = atob(b64);
  } catch {
    return null;
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    return null;
  }

  if (typeof parsed !== "object" || parsed === null) return null;
  const { v, port, token } = parsed as { v?: unknown; port?: unknown; token?: unknown };
  if (v !== 1) return null;
  if (typeof port !== "number" || !Number.isInteger(port) || port < 1 || port > 65535) return null;
  if (typeof token !== "string" || token.length === 0) return null;

  return { port, token };
}
