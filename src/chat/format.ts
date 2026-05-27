// セッション更新時刻の整形と日付グルーピング。
// backend の updated_at は SQLite datetime('now') = "YYYY-MM-DD HH:MM:SS"（UTC）。

export type SessionGroup = "today" | "yesterday" | "earlier";

/** "YYYY-MM-DD HH:MM:SS"(UTC) を Date に。失敗時は現在時刻。 */
export function parseDbTime(s: string): Date {
  const d = new Date(s.replace(" ", "T") + "Z");
  return isNaN(d.getTime()) ? new Date() : d;
}

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** 相対時刻の短い表記（i18n は #18 で。ここは locale 非依存の簡易表記）。 */
export function relativeTime(updatedAt: string, now: Date = new Date()): string {
  const d = parseDbTime(updatedAt);
  const diffSec = Math.max(0, (now.getTime() - d.getTime()) / 1000);
  if (diffSec < 60) return "just now";
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  const dayDiff = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  if (dayDiff <= 0) return `${Math.floor(diffSec / 3600)}h ago`;
  if (dayDiff === 1) return "yesterday";
  if (dayDiff < 7) return `${dayDiff}d ago`;
  return d.toLocaleDateString();
}

/** セッションがどの日付グループに属するか。 */
export function sessionGroup(updatedAt: string, now: Date = new Date()): SessionGroup {
  const d = parseDbTime(updatedAt);
  const dayDiff = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  if (dayDiff <= 0) return "today";
  if (dayDiff === 1) return "yesterday";
  return "earlier";
}
