import type { UiLang } from "./i18n";

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["year", 365 * 24 * 3600],
  ["month", 30 * 24 * 3600],
  ["day", 24 * 3600],
  ["hour", 3600],
  ["minute", 60],
];

/** "há 2 min" / "2 min. ago"; an empty string when the time cannot be read. */
export function relativeTime(iso: string, lang: UiLang, now: number = Date.now()): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const seconds = Math.round((now - then) / 1000);
  const formatter = new Intl.RelativeTimeFormat(lang === "pt" ? "pt-PT" : "en-GB", {
    numeric: "auto",
    style: "short",
  });
  if (seconds < 45) return lang === "pt" ? "agora" : "now";
  for (const [unit, size] of UNITS) {
    if (seconds >= size) return formatter.format(-Math.floor(seconds / size), unit);
  }
  return formatter.format(-Math.max(1, Math.round(seconds / 60)), "minute");
}
