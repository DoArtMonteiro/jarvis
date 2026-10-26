import { describe, expect, it } from "vitest";
import { en, pt } from "./i18n";
import {
  CAPABILITIES,
  approvalExcerptSource,
  argumentLines,
  capabilityKey,
  composerBlock,
  consequenceKey,
  excerpt,
  formatDuration,
  formatTps,
  governorMeta,
  greetingKey,
  holdState,
  inferenceMeta,
  jobGoalKey,
  jobStatusMeta,
  jobSummaryParts,
  replyFailureKey,
} from "./present";
import type { GovernorView, HealthReport, InferenceView, JobView } from "./types";

const inference = (patch: Partial<InferenceView> = {}): InferenceView => ({
  state: "ready",
  profile: "primary",
  model: "model.gguf",
  model_sha256: "0".repeat(64),
  context_size: 8192,
  gpu_layers: 99,
  last_error: null,
  load_ms: 1200,
  last_stats: null,
  retention_days: 0,
  warmup_tokens_per_second_x10: 410,
  slow: false,
  ...patch,
});

const health = (governor: Partial<GovernorView>): HealthReport => ({
  ok: true,
  core_version: "0.2.0",
  rpc_protocol: 4,
  uptime_ms: 1000,
  governor: { state: "open", reason: null, process: null, held_ms: null, overdue: false, ...governor },
  queued_jobs: 0,
  pending_approvals: 0,
});

describe("job status", () => {
  it("gives every status a tone and a word", () => {
    const statuses: JobView["status"][] = ["queued", "running", "completed", "failed", "cancelled", "interrupted"];
    for (const status of statuses) {
      const meta = jobStatusMeta(status);
      expect(pt[meta.key]).toBeTruthy();
      expect(en[meta.key]).toBeTruthy();
    }
    expect(jobStatusMeta("completed").tone).toBe("ok");
    expect(jobStatusMeta("failed").tone).toBe("danger");
    expect(jobStatusMeta("queued").tone).toBe("pending");
  });
});

describe("model and governor state", () => {
  it("marks a slow model as a warning, not as plainly ready", () => {
    expect(inferenceMeta(inference())).toEqual({ tone: "ok", key: "stateReady" });
    expect(inferenceMeta(inference({ slow: true })).tone).toBe("warn");
    expect(inferenceMeta(inference({ state: "failed" })).tone).toBe("danger");
    expect(inferenceMeta(null).key).toBe("stateUnknown");
  });

  it("names the governor's hold by its reason", () => {
    expect(governorMeta(health({}).governor).key).toBe("govOpen");
    expect(governorMeta(health({ state: "held", reason: "game" }).governor)).toEqual({
      tone: "warn",
      key: "govHeldGame",
    });
    expect(governorMeta(health({ state: "held", reason: "starting" }).governor).key).toBe("govHeldStarting");
    expect(governorMeta(null).tone).toBe("mute");
  });

  it("describes a game hold for the global band", () => {
    expect(holdState(health({}), inference())).toBeNull();
    const held = health({ state: "held", reason: "game", process: "cs2.exe", held_ms: 5000 });
    expect(holdState(held, inference({ state: "paused_for_game" }))).toEqual({
      kind: "paused",
      process: "cs2.exe",
      heldMs: 5000,
      overdue: false,
    });
    expect(holdState(held, inference({ profile: "fallback" }))?.kind).toBe("fallback");
    expect(holdState(health({ state: "held", reason: "detection_failed" }), inference())?.kind).toBe("detection");
    expect(holdState(health({ state: "held", reason: "starting" }), inference())).toBeNull();
    const releasing = health({ state: "releasing", reason: null, process: null, held_ms: 9000 });
    expect(holdState(releasing, inference({ state: "paused_for_game" }))?.kind).toBe("resuming");
  });

  it("explains why a composer cannot send", () => {
    expect(composerBlock("ok", inference())).toBeNull();
    expect(composerBlock("unreachable", inference())).toBe("blockOffline");
    expect(composerBlock("ok", inference({ state: "paused_for_game" }))).toBe("blockPaused");
    expect(composerBlock("ok", inference({ state: "disabled" }))).toBe("blockNoModel");
    // A fallback model during a game still answers.
    expect(composerBlock("ok", inference({ profile: "fallback" }))).toBeNull();
  });
});

describe("excerpts", () => {
  it("leaves short text alone and marks a cut", () => {
    expect(excerpt("  Boa   tarde.\nAté já ", 40)).toEqual({ text: "Boa tarde. Até já", cut: false });
    const long = excerpt("a".repeat(100), 10);
    expect(long.cut).toBe(true);
    expect(Array.from(long.text)).toHaveLength(10);
    expect(long.text.endsWith("…")).toBe(true);
  });

  it("counts characters, not UTF-16 units", () => {
    expect(excerpt("😀😀😀", 3)).toEqual({ text: "😀😀😀", cut: false });
  });

  it("prefers the text being written for a compact preview", () => {
    expect(approvalExcerptSource({ action: "Replace", text: "Olá", sha256: "x" })).toBe("Olá");
    expect(approvalExcerptSource({ path: "notes.txt", preview: "hello" })).toBe("hello");
    expect(approvalExcerptSource({})).toBe("");
  });

  it("reads the Core's display escapes in an excerpt without merging them", () => {
    // As the Core shows a newline, a backslash before n, and an escape character.
    expect(approvalExcerptSource({ text: "a\\nb" })).toBe("a b");
    expect(approvalExcerptSource({ text: "a\\\\nb" })).toBe("a\\nb");
    expect(approvalExcerptSource({ text: "x\\u{1b}[2J\\ty" })).toBe("x\\u{1b}[2J y");
    expect(excerpt(approvalExcerptSource({ preview: "one\\r\\ntwo" }), 40).text).toBe("one two");
  });

  it("keeps every argument in full", () => {
    const value = "x".repeat(5000);
    expect(argumentLines({ text: value, lines: "1" })).toBe(`text: ${value}\nlines: 1`);
  });
});

describe("formatting", () => {
  it("formats durations", () => {
    expect(formatDuration(12_400)).toBe("12 s");
    expect(formatDuration(248_000)).toBe("4 min 08 s");
    expect(formatDuration(24_120_000)).toBe("6 h 42 min");
    expect(formatDuration(-5)).toBe("0 s");
  });

  it("writes speeds in the reader's language", () => {
    expect(formatTps(416, "pt")).toBe("41,6");
    expect(formatTps(416, "en")).toBe("41.6");
  });

  it("greets by the hour", () => {
    expect(greetingKey(8)).toBe("greetMorning");
    expect(greetingKey(15)).toBe("greetAfternoon");
    expect(greetingKey(23)).toBe("greetEvening");
    expect(greetingKey(2)).toBe("greetEvening");
  });
});

describe("failed replies", () => {
  it("translates the Core's known reasons", () => {
    expect(replyFailureKey("the reply reached the length limit and was cut short")).toBe("failLength");
    expect(replyFailureKey("the reply took too long and was stopped")).toBe("failTimeout");
    expect(replyFailureKey("the model server stopped before the reply was finished")).toBe("failServer");
    expect(replyFailureKey("something new")).toBeNull();
  });
});

describe("names", () => {
  it("has a consequence for every known tool and a label for every capability", () => {
    for (const tool of [
      "desktop.write_clipboard_text",
      "workspace.write_file",
      "session_agent.healthcheck",
      "filesystem.read_fixture",
      "system.info",
    ]) {
      expect(consequenceKey(tool, "pt"), tool).not.toBeNull();
      expect(consequenceKey(tool, "en"), tool).not.toBeNull();
    }
    expect(consequenceKey("unknown.tool", "pt")).toBeNull();
    expect(CAPABILITIES).toHaveLength(8);
    for (const cap of CAPABILITIES) expect(capabilityKey(cap.id, "pt"), cap.id).not.toBeNull();
  });
});

describe("job presentation", () => {
  it("names a clipboard goal without its payload id", () => {
    expect(jobGoalKey("copy payload:01a1:68b1")).toBe("jobGoalClipboard");
    expect(jobGoalKey("describe the runtime")).toBeNull();
  });

  it("localises the Core's summary and drops zero counts", () => {
    const summary = "2 call(s): completed=1 denied=0 declined=1 expired=0 rejected=0 failed=0";
    expect(jobSummaryParts(summary, "pt")).toEqual(["passos: 2", "concluídos: 1", "recusados por ti: 1"]);
    expect(jobSummaryParts(summary, "en")).toEqual(["steps: 2", "completed: 1", "declined by you: 1"]);
  });

  it("leaves a summary of another shape to be shown as it is", () => {
    expect(jobSummaryParts("something else", "pt")).toBeNull();
    expect(jobSummaryParts("1 call(s): completed=x", "pt")).toBeNull();
  });
});
