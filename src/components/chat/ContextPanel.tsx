// 右パネル（v1, 最小）。#18 で選択中文献・このターンの引用・使用ツール集計を作り込む。
import { useTranslation } from "react-i18next";
import { useChatStore } from "../../chat/store";
import { ChatIcon } from "./ChatIcon";

export function ContextPanel() {
  const { t } = useTranslation();
  const entryIds = useChatStore((s) => s.entryIds);

  return (
    <aside style={{ width: 280, flexShrink: 0, height: "100%", background: "var(--surface)", borderLeft: "1px solid var(--border)", display: "flex", flexDirection: "column", overflow: "hidden" }}>
      <div style={{ padding: "12px 16px 11px", borderBottom: "1px solid var(--border)", display: "flex", alignItems: "center", gap: 8 }}>
        <div style={{ fontSize: 12, fontWeight: 600, color: "var(--text)", display: "flex", alignItems: "center", gap: 6 }}>
          {t("chat.contextTitle")}
          <span style={{ fontSize: 10.5, padding: "1px 6px", borderRadius: 999, background: "var(--surface-2)", color: "var(--text-faint)", fontVariantNumeric: "tabular-nums" }}>{entryIds.length}</span>
        </div>
      </div>
      <div style={{ flex: 1, overflow: "auto", padding: "12px 14px" }}>
        <div style={{ padding: "10px 12px", borderRadius: 7, background: "var(--surface-2)", border: "1px dashed var(--border)", fontSize: 11, color: "var(--text-mute)", lineHeight: 1.55 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
            <ChatIcon name="info" size={11} color="var(--text-mute)" />
            <span style={{ fontWeight: 600, color: "var(--text)" }}>{t("chat.approvalPolicyTitle")}</span>
          </div>
          {t("chat.approvalPolicyBody")}
        </div>
      </div>
    </aside>
  );
}
