// オプションページ: 接続コードを貼り付け → パース → GET /clipper で疎通確認 → 保存。

import { parseConnectCode } from "./connect-code.js";
import { loadConfig, saveConfig, ping } from "./api.js";

function msg(key: string, substitutions?: string[]): string {
  return chrome.i18n.getMessage(key, substitutions) || key;
}

function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing element #${id}`);
  return el;
}

function setStatus(text: string, ok: boolean | null): void {
  const el = $("status");
  el.textContent = text;
  el.style.color = ok === null ? "#5f6368" : ok ? "#188038" : "#d93025";
}

async function init(): Promise<void> {
  // 静的テキストの i18n 適用
  document.title = msg("optionsTitle");
  $("heading").textContent = msg("optionsTitle");
  $("instructions").textContent = msg("optionsInstructions");
  ($("code") as HTMLTextAreaElement).placeholder = "lc1.…";
  $("save").textContent = msg("optionsSave");

  const existing = await loadConfig();
  if (existing) {
    setStatus(msg("optionsPaired", [String(existing.port)]), true);
  }

  $("save").addEventListener("click", () => {
    void (async () => {
      const code = ($("code") as HTMLTextAreaElement).value;
      const cfg = parseConnectCode(code);
      if (!cfg) {
        setStatus(msg("optionsBadCode"), false);
        return;
      }
      setStatus(msg("optionsTesting"), null);
      if (!(await ping(cfg))) {
        setStatus(msg("optionsPingFailed"), false);
        return;
      }
      await saveConfig(cfg);
      setStatus(msg("optionsPaired", [String(cfg.port)]), true);
      ($("code") as HTMLTextAreaElement).value = "";
    })();
  });
}

void init();
