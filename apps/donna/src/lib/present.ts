// Pure presentation helpers: how Core states, jobs and approvals are named,
// toned and shortened on screen. No state, no I/O, so they are unit-tested.

import { type StringKey, type UiLang, dictionaries } from "./i18n";
import type { GovernorView, HealthReport, InferenceView, JobView } from "./types";

/** Semantic tones. Decoration (brass, oxblood) never stands in for these. */
export type Tone = "ok" | "warn" | "danger" | "pending" | "mute";

export interface StatusMeta {
  tone: Tone;
  key: StringKey;
}

const JOB_STATUS: Record<JobView["status"], StatusMeta> = {
  queued: { tone: "pending", key: "jobQueued" },
  running: { tone: "pending", key: "jobRunning" },
  completed: { tone: "ok", key: "jobCompleted" },
  failed: { tone: "danger", key: "jobFailed" },
  cancelled: { tone: "mute", key: "jobCancelled" },
  interrupted: { tone: "warn", key: "jobInterrupted" },
};

export function jobStatusMeta(status: JobView["status"]): StatusMeta {
  return JOB_STATUS[status];
}

export function inferenceMeta(inference: InferenceView | null): StatusMeta {
  if (!inference) return { tone: "mute", key: "stateUnknown" };
  switch (inference.state) {
    case "ready":
      // A working but slow model is a warning, not a plain "ready".
      return { tone: inference.slow ? "warn" : "ok", key: "stateReady" };
    case "generating":
      return { tone: "pending", key: "stateGenerating" };
    case "loading":
      return { tone: "pending", key: "stateLoading" };
    case "paused_for_game":
      return { tone: "warn", key: "statePaused" };
    case "unloaded":
      return { tone: "mute", key: "stateUnloaded" };
    case "failed":
      return { tone: "danger", key: "stateFailed" };
    case "disabled":
      return { tone: "mute", key: "stateDisabled" };
  }
}

export function governorMeta(governor: GovernorView | null | undefined): StatusMeta {
  if (!governor) return { tone: "mute", key: "stateUnknown" };
  switch (governor.state) {
    case "off":
      return { tone: "mute", key: "govOff" };
    case "open":
      return { tone: "ok", key: "govOpen" };
    case "releasing":
      return { tone: "pending", key: "govReleasing" };
    case "held":
      if (governor.reason === "game") return { tone: "warn", key: "govHeldGame" };
      if (governor.reason === "detection_failed") return { tone: "warn", key: "govHeldDetection" };
      return { tone: "pending", key: "govHeldStarting" };
  }
}

export interface HoldState {
  /** paused: the model is stopped for a game; fallback: a smaller model answers; detection: the check failed. */
  kind: "paused" | "resuming" | "fallback" | "detection";
  process: string | null;
  heldMs: number | null;
  overdue: boolean;
}

/** What the global band says about the governor, or null when nothing is held. */
export function holdState(health: HealthReport | null, inference: InferenceView | null): HoldState | null {
  const governor = health?.governor ?? null;
  const base = {
    process: governor?.process ?? null,
    heldMs: governor?.held_ms ?? null,
    overdue: governor?.overdue ?? false,
  };
  if (inference?.state === "paused_for_game") {
    // The game has closed and the governor waits out its resume delay.
    return { kind: governor?.state === "releasing" ? "resuming" : "paused", ...base };
  }
  if (governor?.state !== "held") return null;
  if (governor.reason === "detection_failed") return { kind: "detection", ...base };
  if (governor.reason !== "game") return null;
  return { kind: inference?.profile === "fallback" ? "fallback" : "paused", ...base };
}

/** Why a composer cannot send right now, or null when it can. */
export function composerBlock(core: string, inference: InferenceView | null): StringKey | null {
  if (core !== "ok") return "blockOffline";
  if (inference?.state === "paused_for_game") return "blockPaused";
  if (inference?.state === "disabled") return "blockNoModel";
  return null;
}

/**
 * Shorten text for a preview, collapsing whitespace. `cut` says whether
 * anything was left out, so the screen can say it is an excerpt.
 */
export function excerpt(text: string, max: number): { text: string; cut: boolean } {
  const flat = text.replace(/\s+/g, " ").trim();
  const chars = Array.from(flat);
  if (chars.length <= max) return { text: flat, cut: false };
  return { text: `${chars.slice(0, Math.max(1, max - 1)).join("").trimEnd()}…`, cut: true };
}

/** "12 s", "4 min 08 s", "6 h 42 min". */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  if (total < 60) return `${total} s`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${minutes} min ${String(total % 60).padStart(2, "0")} s`;
  const hours = Math.floor(minutes / 60);
  return `${hours} h ${String(minutes % 60).padStart(2, "0")} min`;
}

const locale = (lang: UiLang): string => (lang === "pt" ? "pt-PT" : "en-GB");

/** Local wall-clock time of an ISO timestamp; empty when it cannot be read. */
export function formatClock(iso: string, lang: UiLang, seconds = false): string {
  const time = Date.parse(iso);
  if (Number.isNaN(time)) return "";
  return new Intl.DateTimeFormat(locale(lang), {
    hour: "2-digit",
    minute: "2-digit",
    second: seconds ? "2-digit" : undefined,
    hourCycle: "h23",
  }).format(time);
}

/** Tenths of tokens per second, written for the reader's language. */
export function formatTps(x10: number, lang: UiLang): string {
  return (x10 / 10).toLocaleString(locale(lang), {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  });
}

export function formatNumber(n: number, lang: UiLang): string {
  return n.toLocaleString(locale(lang));
}

/** A job goal as a person reads it; clipboard goals name only a payload id. */
export function jobGoalKey(goal: string): StringKey | null {
  return goal.startsWith("copy payload:") ? "jobGoalClipboard" : null;
}

const OUTCOMES = ["completed", "denied", "declined", "expired", "rejected", "failed"] as const;

/**
 * The Core's job summary ("2 call(s): completed=1 denied=0 ...") as localised
 * parts, leaving out zero counts. Null when the text has another shape, so the
 * caller shows it unchanged.
 */
export function jobSummaryParts(summary: string, lang: UiLang): string[] | null {
  const m = /^(\d+) call\(s\): (.*)$/.exec(summary.trim());
  const calls = m?.[1];
  const rest = m?.[2];
  if (calls === undefined || rest === undefined) return null;
  const counts = new Map<string, number>();
  for (const pair of rest.split(" ")) {
    const kv = /^([a-z]+)=(\d+)$/.exec(pair);
    const name = kv?.[1];
    const value = kv?.[2];
    if (name === undefined || value === undefined) return null;
    counts.set(name, Number(value));
  }
  const dict = dictionaries[lang];
  const parts = [dict.jobSteps.replace("{n}", calls)];
  for (const outcome of OUTCOMES) {
    const n = counts.get(outcome) ?? 0;
    if (n > 0) parts.push(dict[`jobOutcome_${outcome}` as const].replace("{n}", String(n)));
  }
  return parts;
}

/** Greeting for the hour of the day. */
export function greetingKey(hour: number): StringKey {
  if (hour >= 5 && hour < 12) return "greetMorning";
  if (hour >= 12 && hour < 20) return "greetAfternoon";
  return "greetEvening";
}

/**
 * The Core explains a failed reply in English. Known reasons are shown in
 * the reader's language; anything else is shown as the Core wrote it.
 */
export function replyFailureKey(error: string): StringKey | null {
  const text = error.toLowerCase();
  if (text.includes("length limit")) return "failLength";
  if (text.includes("too long and was cut")) return "failTooLong";
  if (text.includes("took too long")) return "failTimeout";
  if (text.includes("game")) return "failGame";
  if (text.includes("model server stopped")) return "failServer";
  return null;
}

// ---- approvals ----------------------------------------------------------------

const keyFor = (prefix: string, name: string, lang: UiLang): StringKey | null => {
  const key = `${prefix}${name.replace(/[^a-z0-9]/gi, "_")}`;
  return key in dictionaries[lang] ? (key as StringKey) : null;
};

/** The plain-language consequence of a tool, when the app knows the tool. */
export function consequenceKey(tool: string, lang: UiLang): StringKey | null {
  return keyFor("cons_", tool, lang);
}

/** Every argument exactly as the Core returned it, one per line. Never shortened. */
export function argumentLines(args: Record<string, string>): string {
  return Object.entries(args)
    .map(([key, value]) => `${key}: ${value}`)
    .join("\n");
}

const EXCERPT_FIELDS = ["text", "preview", "path", "label", "target", "action"];

/**
 * Read the Core's display escapes for a one-line excerpt: `\n`, `\r` and `\t`
 * become a space and `\\` a backslash, so an escaped backslash followed by `n`
 * still shows as `\n`. Other escapes such as `\u{1b}` stay visible. Only for
 * excerpts; the approval detail shows the arguments exactly as reported.
 */
export function readableEscapes(text: string): string {
  return text.replace(/\\([\\nrt])/g, (_, c: string) => (c === "\\" ? "\\" : " "));
}

/** The most telling argument to excerpt in a compact list. */
export function approvalExcerptSource(args: Record<string, string>): string {
  for (const field of EXCERPT_FIELDS) {
    const value = args[field];
    if (value !== undefined && value.trim() !== "") return readableEscapes(value);
  }
  return argumentLines(args);
}

// ---- capabilities -----------------------------------------------------------------

export interface CapabilityInfo {
  id: string;
  /** Fixed in the Core's code: class A runs without asking, the rest need a person. */
  needsApproval: boolean;
  /** The Core has no tool behind it yet. */
  notYet: boolean;
}

/** The capabilities the Core defines, in the order a person reads them. */
export const CAPABILITIES: CapabilityInfo[] = [
  { id: "system.info", needsApproval: false, notYet: false },
  { id: "filesystem.read.fixture", needsApproval: false, notYet: false },
  { id: "workspace.write", needsApproval: true, notYet: false },
  { id: "session_agent.healthcheck", needsApproval: true, notYet: false },
  { id: "desktop.clipboard.write", needsApproval: true, notYet: false },
  { id: "desktop.screen.observe", needsApproval: true, notYet: true },
  { id: "desktop.clipboard.read", needsApproval: true, notYet: true },
  { id: "desktop.input.send", needsApproval: true, notYet: true },
];

export function capabilityKey(id: string, lang: UiLang): StringKey | null {
  return keyFor("cap_", id, lang);
}
