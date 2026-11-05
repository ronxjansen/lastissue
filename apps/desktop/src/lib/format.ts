// Display helpers: names, relative and absolute dates, labels.

import type { GroupKey, Priority, State } from "./api";

/** `Ada Lovelace <ada@x>` → `Ada Lovelace`; a bare address → its local part. */
export function displayName(addr: string): string {
  const a = addr.trim();
  const lt = a.indexOf("<");
  if (lt >= 0) {
    const name = a.slice(0, lt).trim().replace(/^"|"$/g, "").trim();
    if (name) return name;
  }
  const email = a.replace(/^</, "").replace(/>$/, "");
  return email.split("@")[0] ?? email;
}

/** Compact relative time: `now`, `5m`, `3h`, `2d`, then a date. */
export function ago(ms: number, now = Date.now()): string {
  const s = Math.floor((now - ms) / 1000);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86_400) return `${Math.floor(s / 3600)}h`;
  if (s < 30 * 86_400) return `${Math.floor(s / 86_400)}d`;
  return new Date(ms).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

export function dateTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

export function day(ms: number): string {
  return new Date(ms).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

export const STATES: State[] = [
  "backlog",
  "todo",
  "in-progress",
  "in-review",
  "done",
  "cancelled",
  "duplicate",
];

export const STATE_LABEL: Record<State, string> = {
  backlog: "Backlog",
  todo: "Todo",
  "in-progress": "In progress",
  "in-review": "In review",
  done: "Done",
  cancelled: "Cancelled",
  duplicate: "Duplicate",
};

export const PRIORITY_LABEL: Record<Priority | "none", string> = {
  urgent: "Urgent",
  high: "High",
  medium: "Medium",
  low: "Low",
  none: "No priority",
};

export function groupKeyId(k: GroupKey): string {
  return `${k.kind}:${k.value ?? "none"}`;
}

export const SORT_LABEL = {
  priority: "Priority",
  updated: "Updated",
  created: "Created",
  due: "Due",
  state: "State",
} as const;
