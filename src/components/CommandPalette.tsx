import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Command } from "cmdk";
import { useTranslation } from "react-i18next";
import { useTheme } from "../hooks/useTheme";
import { useLanguage } from "../hooks/useLanguage";
import type { EntrySummary } from "../types";

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  entries: EntrySummary[];
  onSelectEntry: (id: number) => void;
  onOpenDetail: (id: number) => void;
  onNewEntry: () => void;
  onOpenChat: () => void;
  onOpenSettings: () => void;
  onOpenBibtexSync: () => void;
  onSyncBibtexNow: () => void;
  onSelectView: (view: string) => void;
}

export function CommandPalette({
  open, onClose, entries, onSelectEntry, onOpenDetail,
  onNewEntry, onOpenChat, onOpenSettings, onOpenBibtexSync, onSyncBibtexNow,
  onSelectView,
}: CommandPaletteProps) {
  const { t } = useTranslation();
  const { setTheme } = useTheme();
  const { setLanguage } = useLanguage();
  const [search, setSearch] = useState("");
  const [entryResults, setEntryResults] = useState<EntrySummary[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);

  // open のたびに検索文字列をリセット + 入力にフォーカス
  useEffect(() => {
    if (!open) return;
    setSearch("");
    setEntryResults([]);
    // 次フレームで focus（マウント直後だと取りこぼすことがある）
    const id = requestAnimationFrame(() => inputRef.current?.focus());
    return () => cancelAnimationFrame(id);
  }, [open]);

  // 文献検索は backend で全ライブラリを対象にする（CR-028）。
  // 従来は props で渡された読み込み済みの先頭40件だけが対象で、大きな
  // ライブラリでは開いている一覧に無い文献へ palette から辿れなかった。
  useEffect(() => {
    if (!open) return;
    const q = search.trim();
    if (!q) { setEntryResults([]); return; }
    let cancelled = false;
    const timer = setTimeout(() => {
      void (async () => {
        try {
          const res = await invoke<EntrySummary[]>("search_entries", { query: q });
          if (!cancelled) setEntryResults(res.slice(0, 40));
        } catch {
          if (!cancelled) setEntryResults([]);
        }
      })();
    }, 160);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [open, search]);

  // Esc で閉じる
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, onClose]);

  if (!open) return null;

  const run = (fn: () => void) => {
    onClose();
    // 1 フレーム遅らせて閉じた後にアクション実行（モーダル重なりを避ける）
    requestAnimationFrame(fn);
  };

  // 検索文字列がある時は backend 全文献検索の結果、空の時は読み込み済みの最近エントリ。
  const query = search.trim();
  const entryMatches = query ? entryResults : entries.slice(0, 40);

  return (
    <div className="lc-cmdk-overlay" onClick={onClose}>
      <div onClick={e => e.stopPropagation()}>
        <Command className="lc-cmdk" label={t("command.placeholder")}>
          <Command.Input
            ref={inputRef}
            value={search}
            onValueChange={setSearch}
            placeholder={t("command.placeholder")}
          />
          <Command.List>
            <Command.Empty>{t("command.empty")}</Command.Empty>

            <Command.Group heading={t("command.group.actions")}>
              <Command.Item value="new-entry add" onSelect={() => run(onNewEntry)}>
                <span>{t("command.action.newEntry")}</span>
                <span className="lc-cmdk-item-sub">⌘N</span>
              </Command.Item>
              <Command.Item value="chat new chat open assistant" onSelect={() => run(onOpenChat)}>
                <span>{t("command.action.openChat")}</span>
                <span className="lc-cmdk-item-sub">⌘J</span>
              </Command.Item>
              <Command.Item value="settings preferences" onSelect={() => run(onOpenSettings)}>
                <span>{t("command.action.settings")}</span>
              </Command.Item>
              <Command.Item value="bibtex sync settings" onSelect={() => run(onOpenBibtexSync)}>
                <span>{t("command.action.bibtexSync")}</span>
              </Command.Item>
              <Command.Item value="bibtex sync now" onSelect={() => run(onSyncBibtexNow)}>
                <span>{t("command.action.bibtexSyncNow")}</span>
              </Command.Item>
            </Command.Group>

            <Command.Group heading={t("command.group.view")}>
              <Command.Item value="view all entries" onSelect={() => run(() => onSelectView("all"))}>
                <span>{t("command.action.viewAll")}</span>
              </Command.Item>
              <Command.Item value="view starred favorites" onSelect={() => run(() => onSelectView("starred"))}>
                <span>{t("command.action.viewStarred")}</span>
              </Command.Item>
              <Command.Item value="view unfiled" onSelect={() => run(() => onSelectView("unfiled"))}>
                <span>{t("command.action.viewUnfiled")}</span>
              </Command.Item>
              <Command.Item value="view trash" onSelect={() => run(() => onSelectView("trash"))}>
                <span>{t("command.action.viewTrash")}</span>
              </Command.Item>
            </Command.Group>

            <Command.Group heading={t("command.group.theme")}>
              <Command.Item value="theme light" onSelect={() => run(() => setTheme("light"))}>
                <span>{t("command.action.themeLight")}</span>
              </Command.Item>
              <Command.Item value="theme dark" onSelect={() => run(() => setTheme("dark"))}>
                <span>{t("command.action.themeDark")}</span>
              </Command.Item>
              <Command.Item value="theme auto system" onSelect={() => run(() => setTheme("auto"))}>
                <span>{t("command.action.themeAuto")}</span>
              </Command.Item>
            </Command.Group>

            <Command.Group heading={t("command.group.language")}>
              <Command.Item value="language japanese ja 日本語" onSelect={() => run(() => setLanguage("ja"))}>
                <span>{t("command.action.languageJa")}</span>
              </Command.Item>
              <Command.Item value="language english en" onSelect={() => run(() => setLanguage("en"))}>
                <span>{t("command.action.languageEn")}</span>
              </Command.Item>
            </Command.Group>

            <Command.Group heading={t("command.group.entries")}>
              {entryMatches.map(e => {
                const authors = e.authors.map(a => a.name).join(" ");
                const tagsText = e.tags.map(t => t.name).join(" ");
                const yearText = e.year ? String(e.year) : "";
                // backend が abstract/識別子などタイトルに含まれない箇所でヒットさせても
                // cmdk のローカル fuzzy filter で消えないよう、検索語を value に含める。
                // 末尾に id を付けて value を一意にする（cmdk は value を同一性に使う）。
                const value = `${query} ${e.title} ${authors} ${tagsText} ${yearText} ${e.id}`;
                return (
                  <Command.Item
                    key={e.id}
                    value={value}
                    onSelect={() => run(() => { onSelectEntry(e.id); onOpenDetail(e.id); })}
                  >
                    <span style={{
                      flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
                    }}>{e.title}</span>
                    {e.year && (
                      <span className="lc-cmdk-item-sub">{e.year}</span>
                    )}
                  </Command.Item>
                );
              })}
            </Command.Group>
          </Command.List>
        </Command>
      </div>
    </div>
  );
}
