// LLM プロバイダごとのモデルプリセットと既定モデル。設定 / 新規 Chat ダイアログで共用。
import type { LlmProvider } from "../types";

export const MODEL_PRESETS: Record<LlmProvider, { id: string; label: string }[]> = {
  openai: [
    { id: "gpt-4o-mini", label: "gpt-4o-mini" },
    { id: "gpt-4o", label: "gpt-4o" },
    { id: "gpt-4.1-mini", label: "gpt-4.1-mini" },
    { id: "gpt-4.1", label: "gpt-4.1" },
    { id: "o4-mini", label: "o4-mini" },
  ],
  anthropic: [
    { id: "claude-haiku-4-5-20251001", label: "Claude Haiku 4.5" },
    { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6" },
    { id: "claude-opus-4-7", label: "Claude Opus 4.7" },
  ],
};

export function defaultModelFor(provider: LlmProvider): string {
  return MODEL_PRESETS[provider][0].id;
}
