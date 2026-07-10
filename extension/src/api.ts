// LumenCite ローカル API（/clipper）との通信と設定の保存。

import type { ClipPayload, ClipResponse, ClipperConfig } from "./types.js";

const STORAGE_KEY = "clipperConfig";

export async function loadConfig(): Promise<ClipperConfig | null> {
  const stored = await chrome.storage.local.get(STORAGE_KEY);
  const cfg = stored[STORAGE_KEY] as ClipperConfig | undefined;
  return cfg && typeof cfg.port === "number" && typeof cfg.token === "string" ? cfg : null;
}

export async function saveConfig(config: ClipperConfig): Promise<void> {
  await chrome.storage.local.set({ [STORAGE_KEY]: config });
}

function baseUrl(cfg: ClipperConfig): string {
  return `http://127.0.0.1:${cfg.port}/clipper`;
}

/** localhost 応答が無いまま無限に待たないための締切付き fetch（CR-029）。 */
const REQUEST_TIMEOUT_MS = 10_000;

async function fetchWithTimeout(url: string, init: RequestInit, ms = REQUEST_TIMEOUT_MS): Promise<Response> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ms);
  try {
    return await fetch(url, { ...init, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

/** クリップ結果。fetch 不能（アプリ未起動等）は kind: "unreachable"。 */
export type ClipOutcome =
  | { kind: "response"; status: number; body: ClipResponse }
  | { kind: "unreachable" };

export async function ping(cfg: ClipperConfig): Promise<boolean> {
  try {
    const resp = await fetchWithTimeout(baseUrl(cfg), {
      headers: { Authorization: `Bearer ${cfg.token}` },
    });
    if (!resp.ok) return false;
    const body = (await resp.json()) as { ok?: boolean };
    return body.ok === true;
  } catch {
    return false;
  }
}

export async function clip(cfg: ClipperConfig, payload: ClipPayload): Promise<ClipOutcome> {
  try {
    const resp = await fetchWithTimeout(baseUrl(cfg), {
      method: "POST",
      headers: {
        Authorization: `Bearer ${cfg.token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
    });
    let body: ClipResponse;
    try {
      body = (await resp.json()) as ClipResponse;
    } catch {
      body = { status: "error", message: `HTTP ${resp.status}` };
    }
    return { kind: "response", status: resp.status, body };
  } catch {
    return { kind: "unreachable" };
  }
}
