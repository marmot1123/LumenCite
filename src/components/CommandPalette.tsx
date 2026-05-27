import { useEffect, useRef, useState } from "react";
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
  const inputRef = useRef<HTMLInputElement>(null);

  // open のたびに検索文字列をリセット + 入力にフォーカス
  useEffect(() => {
    if (!open) return;
    setSearch("");
    // 次フレームで focus（マウント直後だと取りこぼすことがある）
    const id = requestAnimationFrame(() => inputRef.current?.focus());
    return () => cancelAnimationFrame(id);
  }, [open]);

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

  // タイトル / 著者 / タグから検索（cmdk のデフォルト fuzzy match に渡す value にまとめる）
  const entryMatches = entries.slice(0, 40);

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
                const value = `${e.title} ${authors} ${tagsText} ${yearText}`;
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
