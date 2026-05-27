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
    channel.onmessage = (ev) => set((s) => applyStreamEvent(s, ev));

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
