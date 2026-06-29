// Chat 機能の zustand ストア。セッション一覧・アクティブセッションの会話・
// ストリーミング状態を保持し、Tauri コマンド（invoke / Channel）を呼ぶ。
// 純粋な状態遷移は ./messages に切り出してある。

import { create } from "zustand";
import { invoke, Channel } from "@tauri-apps/api/core";
import type {
  ChatSession,
  ChatStreamEvent,
  ScopeMode,
  SessionWithMessages,
  UiChatMessage,
  UiToolCall,
} from "../types";
import {
  applyApproval,
  applyStreamEvent,
  rowsToUiMessages,
  type ChatMessagesState,
} from "./messages";

export interface NewSessionArgs {
  title: string;
  provider: string;
  model: string;
  scopeMode: ScopeMode;
  entryIds: number[];
}

interface ChatStore extends ChatMessagesState {
  sessions: ChatSession[];
  activeSessionId: number | null;
  /** アクティブセッションのスコープ対象 entry 群 */
  entryIds: number[];
  loadingSessions: boolean;
  /** 直近にアーカイブしたセッション（取り消しトースト表示用）。 */
  archiveToast: { sessionId: number; title: string } | null;
  /** ライブラリ DB を変更し得るツールが実行されるたびに増えるカウンタ。
   *  App 側がこれを購読し、チャット中でも一覧をリアルタイム再読込する。 */
  dataVersion: number;

  loadSessions: () => Promise<void>;
  openSession: (id: number) => Promise<void>;
  createSession: (args: NewSessionArgs) => Promise<ChatSession>;
  sendMessage: (text: string) => Promise<void>;
  approveToolCall: (callId: string, approved: boolean) => Promise<void>;
  cancelStream: () => Promise<void>;
  archiveSession: (id: number) => Promise<void>;
  undoArchive: () => Promise<void>;
  dismissArchiveToast: () => void;
  renameSession: (id: number, title: string) => Promise<void>;
  setScope: (scopeMode: ScopeMode, entryIds: number[]) => Promise<void>;
  setSessionModel: (provider: string, model: string) => Promise<void>;
  /** 最初のターン後に自動タイトル生成。失敗は握り潰す。 */
  maybeGenerateTitle: (id: number) => Promise<void>;
  reset: () => void;
}

export const useChatStore = create<ChatStore>((set, get) => ({
  // ChatMessagesState
  messages: [],
  streaming: false,
  blocking: false,
  pendingApprovals: [],
  error: null,
  // ChatStore
  sessions: [],
  activeSessionId: null,
  entryIds: [],
  loadingSessions: false,
  archiveToast: null,
  dataVersion: 0,

  loadSessions: async () => {
    set({ loadingSessions: true });
    try {
      const sessions = await invoke<ChatSession[]>("list_chat_sessions", {});
      set({ sessions });
    } finally {
      set({ loadingSessions: false });
    }
  },

  openSession: async (id) => {
    const swm = await invoke<SessionWithMessages>("get_chat_session", { id });
    set({
      activeSessionId: id,
      messages: rowsToUiMessages(swm.messages),
      entryIds: swm.entry_ids,
      streaming: false,
      blocking: false,
      pendingApprovals: [],
      error: null,
    });
  },

  createSession: async ({ title, provider, model, scopeMode, entryIds }) => {
    const session = await invoke<ChatSession>("create_chat_session", {
      title,
      provider,
      model,
      scopeMode,
      entryIds,
    });
    set((s) => ({
      sessions: [session, ...s.sessions],
      activeSessionId: session.id,
      entryIds,
      messages: [],
      streaming: false,
      blocking: false,
      pendingApprovals: [],
      error: null,
    }));
    return session;
  },

  sendMessage: async (text) => {
    const sid = get().activeSessionId;
    if (sid == null || !text.trim() || get().streaming) return;

    // 楽観的に user メッセージを追加してストリーミング開始
    const userMessage: UiChatMessage = { role: "user", content: text, tool_calls: [] };
    set((s) => ({ messages: [...s.messages, userMessage], streaming: true, error: null }));

    const channel = new Channel<ChatStreamEvent>();
    channel.onmessage = (ev) => {
      set((s) => applyStreamEvent(s, ev));
      // 書き込みツールが実行された瞬間に dataVersion を進め、一覧の再読込を促す。
      // set は同期なので、ここで get() すれば適用済みの messages を参照できる。
      if (ev.kind === "tool_call_executed") {
        const tc = findToolCallByCallId(get().messages, ev.call_id);
        // 拒否されたカード（state="rejected"）は DB を変更していないので除外する。
        if (tc && tc.state === "done" && isLibraryMutatingTool(tc.tool_name)) {
          set((s) => ({ dataVersion: s.dataVersion + 1 }));
        }
      }
    };

    try {
      await invoke("chat_send_message", { sessionId: sid, userText: text, channel });
    } catch (e) {
      // invoke の reject は Channel の error イベントでも届くが、保険で握る
      set((s) => (s.error ? s : { streaming: false, error: String(e) }));
    } finally {
      set({ streaming: false });
      // セッション一覧の更新日時順を反映するため再読込
      void get().loadSessions();
      void get().maybeGenerateTitle(sid);
    }
  },

  approveToolCall: async (callId, approved) => {
    set((s) => applyApproval(s, callId, approved));
    try {
      await invoke("approve_tool_call", { callId, approved });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  cancelStream: async () => {
    const sid = get().activeSessionId;
    if (sid == null) return;
    try {
      await invoke("cancel_chat_stream", { sessionId: sid });
    } finally {
      set({ blocking: false, pendingApprovals: [] });
    }
  },

  archiveSession: async (id) => {
    const title = get().sessions.find((x) => x.id === id)?.title ?? "";
    await invoke("archive_chat_session", { id });
    set((s) => {
      const sessions = s.sessions.filter((x) => x.id !== id);
      const closing = s.activeSessionId === id;
      return {
        sessions,
        activeSessionId: closing ? null : s.activeSessionId,
        messages: closing ? [] : s.messages,
        archiveToast: { sessionId: id, title },
      };
    });
  },

  undoArchive: async () => {
    const toast = get().archiveToast;
    if (!toast) return;
    set({ archiveToast: null });
    try {
      await invoke("unarchive_chat_session", { id: toast.sessionId });
      await get().loadSessions();
      await get().openSession(toast.sessionId);
    } catch (e) {
      set({ error: String(e) });
    }
  },

  dismissArchiveToast: () => set({ archiveToast: null }),

  renameSession: async (id, title) => {
    const updated = await invoke<ChatSession>("update_chat_session_title", { id, title });
    set((s) => ({ sessions: s.sessions.map((x) => (x.id === id ? updated : x)) }));
  },

  setScope: async (scopeMode, entryIds) => {
    const sid = get().activeSessionId;
    if (sid == null) return;
    const updated = await invoke<ChatSession>("set_chat_session_scope", {
      id: sid,
      scopeMode,
      entryIds,
    });
    set((s) => ({
      sessions: s.sessions.map((x) => (x.id === sid ? updated : x)),
      entryIds,
    }));
  },

  setSessionModel: async (provider, model) => {
    const sid = get().activeSessionId;
    if (sid == null) return;
    const updated = await invoke<ChatSession>("set_chat_session_model", { id: sid, provider, model });
    set((s) => ({ sessions: s.sessions.map((x) => (x.id === sid ? updated : x)) }));
  },

  maybeGenerateTitle: async (id) => {
    const session = get().sessions.find((x) => x.id === id);
    // 既にユーザー/LLM が付けたタイトルがあるならスキップ（既定の "New chat" 等のみ生成）
    if (session && session.title && !isDefaultTitle(session.title)) return;
    try {
      const title = await invoke<string>("generate_chat_title", { sessionId: id });
      set((s) => ({
        sessions: s.sessions.map((x) => (x.id === id ? { ...x, title } : x)),
      }));
    } catch {
      // タイトル生成失敗はサイレントに無視
    }
  },

  reset: () =>
    set({
      messages: [],
      streaming: false,
      blocking: false,
      pendingApprovals: [],
      error: null,
      activeSessionId: null,
      entryIds: [],
    }),
}));

function isDefaultTitle(title: string): boolean {
  const t = title.trim().toLowerCase();
  return t === "" || t === "new chat" || t === "untitled" || t === "新しい chat";
}

/** call_id に対応するツールカードを messages から探す（新しい順）。 */
function findToolCallByCallId(
  messages: UiChatMessage[],
  callId: string,
): UiToolCall | undefined {
  for (let i = messages.length - 1; i >= 0; i--) {
    const tc = messages[i].tool_calls.find((c) => c.call_id === callId);
    if (tc) return tc;
  }
  return undefined;
}

/** ライブラリ一覧（entries）の表示に影響し得る書き込みツールか。
 *  read 系（get_entry / fulltext_search / list_*）と、ローカル DB を変えない
 *  外部連携 mcp_* は除外する。それ以外（create/update/delete/add_tag 等）は
 *  一覧へ反映すべき書き込みとみなす。承認ポリシーの分類（approval.rs）と対応。 */
export function isLibraryMutatingTool(toolName: string): boolean {
  if (
    toolName === "get_entry" ||
    toolName === "fulltext_search" ||
    toolName.startsWith("list_") ||
    toolName.startsWith("mcp_")
  ) {
    return false;
  }
  return true;
}
