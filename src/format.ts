/** Up to two decimals, without trailing zeros: 5, 1.5, 12.25. */
export function formatNumber(value: number): string {
  return String(Math.round(value * 100) / 100);
}

/** "2d 3h 14m", "3h 14m", "14m", or "45s" when under a minute. */
export function formatCountdown(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const days = Math.floor(s / 86400);
  const hours = Math.floor((s % 86400) / 3600);
  const minutes = Math.floor((s % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m`;
  return `${s}s`;
}

/** "just now", "40s ago", "3 min ago", "2 h ago". */
export function formatAgo(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 10) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  return `${Math.floor(s / 3600)} h ago`;
}

/** Local reset time: time only when it is today, otherwise with the day. */
export function formatLocalTime(epochSeconds: number): string {
  const time = new Date(epochSeconds * 1000);
  const sameDay = time.toDateString() === new Date().toDateString();
  return time.toLocaleString(
    undefined,
    sameDay
      ? { hour: "numeric", minute: "2-digit" }
      : { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" },
  );
}

export type IntervalUnit = "seconds" | "minutes" | "hours";

export const UNIT_SECONDS: Record<IntervalUnit, number> = {
  seconds: 1,
  minutes: 60,
  hours: 3600,
};

/** Split a number of seconds into the largest unit that divides it evenly. */
export function splitInterval(seconds: number): { value: number; unit: IntervalUnit } {
  if (seconds % 3600 === 0) return { value: seconds / 3600, unit: "hours" };
  if (seconds % 60 === 0) return { value: seconds / 60, unit: "minutes" };
  return { value: seconds, unit: "seconds" };
}

/** Whole seconds for a value and unit, or null if it is under 1 second. */
export function joinInterval(value: number, unit: IntervalUnit): number | null {
  const seconds = value * UNIT_SECONDS[unit];
  // Check before rounding, so that 0.5 seconds is rejected, not rounded up.
  return Number.isFinite(seconds) && seconds >= 1 ? Math.round(seconds) : null;
}
