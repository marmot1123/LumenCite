// Chat メッセージの純粋ロジック。ストリームイベントの適用と履歴行の復元。
// 副作用なし（ストアや invoke に依存しない）ので単体で検証しやすい。

import type {
  ChatMessageRow,
  ChatStreamEvent,
  ToolCallSpec,
  UiChatMessage,
  UiToolCall,
} from "../types";

/** applyStreamEvent が読み書きする会話まわりの状態スライス。 */
export interface ChatMessagesState {
  messages: UiChatMessage[];
  streaming: boolean;
  /** 承認待ちで composer をブロックしているか */
  blocking: boolean;
  /** 承認待ちの call_id 群 */
  pendingApprovals: string[];
  error: string | null;
}

export const EMPTY_MESSAGES_STATE: ChatMessagesState = {
  messages: [],
  streaming: false,
  blocking: false,
  pendingApprovals: [],
  error: null,
};

/** 末尾が assistant メッセージになるよう保証する（無ければ空で push）。 */
function ensureTrailingAssistant(messages: UiChatMessage[]): UiChatMessage[] {
  const last = messages[messages.length - 1];
  if (last && last.role === "assistant") return messages;
  messages.push({ role: "assistant", content: "", tool_calls: [], streaming: false });
  return messages;
}

/** agentic ループの 1 イベントを会話状態に適用する。 */
export function applyStreamEvent(
  s: ChatMessagesState,
  ev: ChatStreamEvent,
): ChatMessagesState {
  switch (ev.kind) {
    case "session_started":
      return { ...s, streaming: true, error: null };

    case "delta": {
      const messages = [...s.messages];
      const last = messages[messages.length - 1];
      if (last && last.role === "assistant" && last.streaming) {
        messages[messages.length - 1] = { ...last, content: last.content + ev.text };
      } else {
        messages.push({ role: "assistant", content: ev.text, tool_calls: [], streaming: true });
      }
      return { ...s, messages, streaming: true };
    }

    case "tool_call_proposed": {
      const messages = ensureTrailingAssistant([...s.messages]);
      const idx = messages.length - 1;
      const tc: UiToolCall = {
        call_id: ev.call_id,
        tool_name: ev.tool_name,
        args_preview: ev.args_preview,
        needs_approval: ev.needs_approval,
        state: ev.needs_approval ? "needs_approval" : "running",
      };
      messages[idx] = { ...messages[idx], tool_calls: [...messages[idx].tool_calls, tc] };
      const pendingApprovals = ev.needs_approval
        ? [...s.pendingApprovals, ev.call_id]
        : s.pendingApprovals;
      return { ...s, messages, pendingApprovals, blocking: pendingApprovals.length > 0 };
    }

    case "tool_call_executed": {
      const messages = s.messages.map((m) => ({
        ...m,
        tool_calls: m.tool_calls.map((tc) =>
          tc.call_id === ev.call_id
            ? {
                ...tc,
                // 拒否済みのカードは done に上書きしない
                state: (tc.state === "rejected" ? "rejected" : "done") as UiToolCall["state"],
                result_summary: ev.result_summary,
              }
            : tc,
        ),
      }));
      return { ...s, messages };
    }

    case "message_persisted": {
      if (ev.role === "assistant") {
        const messages = [...s.messages];
        const idx = messages.length - 1;
        const last = messages[idx];
        if (last && last.role === "assistant" && (last.streaming || last.id === undefined)) {
          messages[idx] = { ...last, id: ev.message_id, streaming: false };
        } else {
          // テキスト無し（ツールのみ）ターン: 新しい assistant メッセージを作る
          messages.push({
            id: ev.message_id,
            role: "assistant",
            content: "",
            tool_calls: [],
            streaming: false,
          });
        }
        return { ...s, messages };
      }
      if (ev.role === "user") {
        const messages = [...s.messages];
        for (let i = messages.length - 1; i >= 0; i--) {
          if (messages[i].role === "user" && messages[i].id === undefined) {
            messages[i] = { ...messages[i], id: ev.message_id };
            break;
          }
        }
        return { ...s, messages };
      }
      // tool メッセージはカードに畳むので独立行は作らない
      return s;
    }

    case "done":
      return { ...s, streaming: false };

    case "error":
      return { ...s, streaming: false, error: ev.message };
  }
}

/** UI 側で承認/拒否が確定したときのローカル更新。 */
export function applyApproval(
  s: ChatMessagesState,
  callId: string,
  approved: boolean,
): ChatMessagesState {
  const pendingApprovals = s.pendingApprovals.filter((id) => id !== callId);
  const messages = s.messages.map((m) => ({
    ...m,
    tool_calls: m.tool_calls.map((tc) =>
      tc.call_id === callId
        ? { ...tc, state: (approved ? "running" : "rejected") as UiToolCall["state"] }
        : tc,
    ),
  }));
  return { ...s, messages, pendingApprovals, blocking: pendingApprovals.length > 0 };
}

/** セッションを開いたときに、DB の行から会話の UI モデルを復元する。
 *  tool 行は対応する assistant のツールカードへ畳む。 */
export function rowsToUiMessages(rows: ChatMessageRow[]): UiChatMessage[] {
  const out: UiChatMessage[] = [];
  for (const r of rows) {
    if (r.role === "tool") {
      const tc = findToolCall(out, r.tool_call_id);
      if (tc) {
        tc.state = "done";
        tc.result_summary = r.content;
      }
      continue;
    }
    let toolCalls: UiToolCall[] = [];
    if (r.role === "assistant" && r.tool_calls) {
      try {
        const specs = JSON.parse(r.tool_calls) as ToolCallSpec[];
        toolCalls = specs.map((sp) => ({
          call_id: sp.call_id,
          tool_name: sp.tool_name,
          args_preview: JSON.stringify(sp.arguments),
          needs_approval: false,
          state: "done" as const,
        }));
      } catch {
        // 壊れた JSON は無視
      }
    }
    out.push({ id: r.id, role: r.role, content: r.content, tool_calls: toolCalls });
  }
  return out;
}

function findToolCall(messages: UiChatMessage[], callId: string | null): UiToolCall | undefined {
  if (!callId) return undefined;
  for (let i = messages.length - 1; i >= 0; i--) {
    const tc = messages[i].tool_calls.find((c) => c.call_id === callId);
    if (tc) return tc;
  }
  return undefined;
}
