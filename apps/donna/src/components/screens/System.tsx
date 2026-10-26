import type { ComponentChildren } from "preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../../lib/api";
import type { LangSetting } from "../../lib/i18n";
import {
  formatDuration,
  formatNumber,
  formatTps,
  governorMeta,
  inferenceMeta,
} from "../../lib/present";
import { countAffected, needsConfirmation } from "../../lib/retention";
import {
  deleteAllConversations,
  deleteConversation,
  exportConversation,
  loadModel,
  setContextSize,
  setLanguage,
  setRetention,
  t,
  unloadModel,
  useStore,
} from "../../lib/store";
import type { ConversationSummary } from "../../lib/types";
import { Dialog } from "../Dialog";
import { DownloadIcon, LockIcon, TrashIcon } from "../Icons";
import { ModelNotice } from "../ModelNotice";
import { KV, ScreenHead, Section, useNow } from "../ui";

const CONTEXT_SIZES = [4096, 8192, 16384, 32768];
const RETENTION_DAYS = [0, 7, 30, 90, 365];

export function System() {
  return (
    <div class="screen-wrap">
      <ScreenHead eyebrow={t("navSystem")} title={t("systemTitle")} lede={t("systemLede")} />
      <div class="two even">
        <div class="col">
          <CoreFacts />
          <GovernorFacts />
          <Privacy />
        </div>
        <div class="col">
          <ModelFacts />
        </div>
      </div>
      <Settings />
    </div>
  );
}

function CoreFacts() {
  const s = useStore();
  const health = s.health;
  const unknown = t("unknownValue");
  return (
    <Section id="sys-core" class="first" title={t("coreSection")}>
      <dl class="kvs">
        <KV label={t("kvState")} tone={s.core === "ok" ? "ok" : "danger"} mono={false}>
          {s.core === "ok" ? t("coreOk") : t("coreLost")}
        </KV>
        <KV label={t("kvVersion")}>{health?.core_version ?? unknown}</KV>
        <KV label={t("kvProtocol")}>{health ? String(health.rpc_protocol) : unknown}</KV>
        <KV label={t("kvUptime")}>{health ? formatDuration(health.uptime_ms) : unknown}</KV>
        <KV label={t("kvQueued")}>{health ? formatNumber(health.queued_jobs, s.lang) : unknown}</KV>
        <KV label={t("kvPending")}>{health ? formatNumber(health.pending_approvals, s.lang) : unknown}</KV>
      </dl>
    </Section>
  );
}

function ModelFacts() {
  const s = useStore();
  const inference = s.inference;
  const meta = inferenceMeta(inference);
  const unknown = t("unknownValue");
  const stats = inference?.last_stats ?? null;
  return (
    <Section id="sys-model" class="first" title={t("modelSection")}>
      {!inference ? (
        <p class="empty-line">{s.inferenceError ?? t("stateUnknown")}</p>
      ) : (
        <>
          <dl class="kvs">
            <KV label={t("kvState")} tone={meta.tone} mono={false}>
              {t(meta.key)}
            </KV>
            <KV label={t("modelProfile")} mono={false}>
              {inference.profile === "primary" ? t("profilePrimary") : t("profileFallback")}
            </KV>
            <KV label={t("modelName")}>{inference.model}</KV>
            <KV label={t("contextHeading")}>
              {t("contextTokens", { n: formatNumber(inference.context_size, s.lang) })}
            </KV>
            <KV label={t("kvGpuLayers")}>
              {inference.gpu_layers > 0
                ? t("gpuLayersValue", { n: inference.gpu_layers })
                : t("cpuOnlyValue")}
            </KV>
            <KV label={t("modelLoadTime")}>
              {inference.load_ms !== null
                ? t("loadSeconds", { n: formatTps(Math.round(inference.load_ms / 100), s.lang) })
                : unknown}
            </KV>
            <KV label={t("kvLastReply")}>
              {stats
                ? t("statsLine", {
                    tps: formatTps(stats.tokens_per_second_x10, s.lang),
                    ms: stats.first_token_ms,
                  })
                : unknown}
            </KV>
            <KV label={t("warmupSpeed")}>
              {inference.warmup_tokens_per_second_x10 !== null
                ? t("tokensPerSecond", { n: formatTps(inference.warmup_tokens_per_second_x10, s.lang) })
                : unknown}
            </KV>
            <KV label={t("kvSlow")} tone={inference.slow ? "warn" : undefined} mono={false}>
              {inference.slow ? t("slowYes") : t("no")}
            </KV>
            <div class="kv kv-wide">
              <dt>{t("modelSha")}</dt>
              <dd class="mono sha">{inference.model_sha256}</dd>
            </div>
          </dl>
          <ModelNotice inference={inference} />
        </>
      )}
    </Section>
  );
}

function GovernorFacts() {
  const s = useStore();
  const now = useNow();
  const governor = s.health?.governor ?? null;
  const meta = governorMeta(governor);
  const unknown = t("unknownValue");
  const reasons = { game: "reasonGame", starting: "reasonStarting", detection_failed: "reasonDetection" } as const;
  const held = governor?.held_ms !== null && governor?.held_ms !== undefined;
  return (
    <Section id="sys-governor" title={t("governorSection")}>
      <dl class="kvs">
        <KV label={t("kvState")} tone={meta.tone} mono={false}>
          {t(meta.key)}
        </KV>
        <KV label={t("kvReason")} mono={false}>
          {governor?.reason ? t(reasons[governor.reason]) : unknown}
        </KV>
        {governor?.process && <KV label={t("kvProcess")}>{governor.process}</KV>}
        <KV label={t("kvHeldFor")}>
          {held && governor?.held_ms != null
            ? formatDuration(governor.held_ms + Math.max(0, now - s.healthAt))
            : unknown}
        </KV>
        <KV label={t("kvOverdue")} tone={governor?.overdue ? "warn" : undefined} mono={false}>
          {governor ? (governor.overdue ? t("yes") : t("no")) : unknown}
        </KV>
      </dl>
    </Section>
  );
}

function Privacy() {
  const s = useStore();
  const days = s.inference?.retention_days;
  return (
    <Section id="sys-privacy" title={t("privacySection")}>
      <p class="privacy-line">
        <LockIcon />
        <span>{t("privacyLocal")}</span>
      </p>
      <dl class="kvs">
        <KV label={t("retentionHeading")} mono={false}>
          {days === undefined
            ? t("unknownValue")
            : days === 0
              ? t("retentionValueKeep")
              : t("retentionDays", { n: days })}
        </KV>
      </dl>
    </Section>
  );
}

function Settings() {
  const s = useStore();
  const [confirming, setConfirming] = useState(false);
  const [typed, setTyped] = useState("");
  const [pending, setPending] = useState<ConversationSummary | null>(null);
  const [retentionPrompt, setRetentionPrompt] = useState<{ days: number; count: number } | null>(null);
  const [appVersion, setAppVersion] = useState("—");
  useEffect(() => {
    api
      .appInfo()
      .then((info) => setAppVersion(info.app_version))
      .catch(() => {});
  }, []);

  // A shorter period deletes at once: count what would go and ask first.
  const chooseRetention = async (days: number) => {
    const current = s.inference?.retention_days ?? 0;
    if (!needsConfirmation(current, days)) return void setRetention(days);
    try {
      const all = await api.listConversations(null, 200);
      setRetentionPrompt({ days, count: countAffected(all, days) });
    } catch {
      // Without a count the change is not applied silently.
      setRetentionPrompt({ days, count: countAffected(s.conversations, days) });
    }
  };

  const inference = s.inference;
  const offline = s.core !== "ok";
  const canLoad = inference && (inference.state === "unloaded" || inference.state === "failed");
  const canUnload = inference && (inference.state === "ready" || inference.state === "generating");
  const word = t("deleteAllWord");
  const languages: [LangSetting, string][] = [
    ["pt_pt", t("langPt")],
    ["en", t("langEn")],
    ["auto", t("langAuto")],
  ];

  return (
    <Section id="settings-title" title={t("settingsTitle")} class="settings">
      <div class="sets">
        <Setting title={t("languageHeading")} help={t("languageHelp")}>
          <Choice
            label={t("languageHeading")}
            options={languages.map(([value, text]) => ({ value, text }))}
            current={s.setting}
            onChoose={(value) => setLanguage(value as LangSetting)}
          />
        </Setting>

        <Setting title={t("contextHeading")} help={t("contextHelp")}>
          <Choice
            label={t("contextHeading")}
            options={CONTEXT_SIZES.map((size) => ({ value: String(size), text: formatNumber(size, s.lang) }))}
            current={inference ? String(inference.context_size) : ""}
            disabled={!inference || inference.state === "disabled" || offline}
            onChoose={(value) => void setContextSize(Number(value))}
          />
          {s.contextError && (
            <p class="inline-error" role="alert">
              {s.contextError}
            </p>
          )}
        </Setting>

        <Setting title={t("modelControlsHeading")} help={t("modelControlsHelp")}>
          <div class="actions">
            <button type="button" class="btn" disabled={!canLoad || offline} onClick={() => void loadModel()}>
              {t("loadModel")}
            </button>
            <button type="button" class="btn" disabled={!canUnload || offline} onClick={() => void unloadModel()}>
              {t("unloadModel")}
            </button>
          </div>
          <p class="hint">{t("unloadHint")}</p>
        </Setting>

        <Setting title={t("storageHeading")} help={t("storageBody")}>
          <p class="field-label" id="retention-label">
            {t("retentionHeading")}
          </p>
          <p class="hint">{t("retentionHelp")}</p>
          <Choice
            label={t("retentionHeading")}
            options={RETENTION_DAYS.map((days) => ({
              value: String(days),
              text: days === 0 ? t("retentionKeep") : t("retentionDays", { n: days }),
            }))}
            current={inference ? String(inference.retention_days) : ""}
            disabled={!inference || offline}
            onChoose={(value) => void chooseRetention(Number(value))}
          />
        </Setting>

        <Setting title={t("conversationsHeading")}>
          {s.conversations.length === 0 ? (
            <p class="hint">{t("conversationsEmpty")}</p>
          ) : (
            <ul class="conv-admin">
              {s.conversations.map((c) => (
                <li key={c.conversation_id}>
                  <span class="conv-admin-title">{c.title.trim() || t("untitled")}</span>
                  <button
                    type="button"
                    class="btn ghost"
                    disabled={offline}
                    onClick={() => void exportConversation(c.conversation_id)}
                  >
                    <DownloadIcon />
                    {t("exportConversation")}
                  </button>
                  <button
                    type="button"
                    class="btn icon"
                    disabled={offline}
                    aria-label={`${t("deleteConversation")}: ${c.title.trim() || t("untitled")}`}
                    onClick={() => setPending(c)}
                  >
                    <TrashIcon />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </Setting>

        <Setting title={t("clipboardHeading")} help={t("clipboardBody")} />

        <Setting title={t("dangerHeading")} help={t("deleteAllHelp")} danger>
          <div class="actions">
            <button
              type="button"
              class="btn danger"
              disabled={offline}
              onClick={() => {
                setTyped("");
                setConfirming(true);
              }}
            >
              {t("deleteAll")}
            </button>
          </div>
        </Setting>
      </div>

      <p class="hint about">{t("aboutVersion", { app: appVersion, core: s.health?.core_version ?? "—" })}</p>

      <Dialog open={retentionPrompt !== null} onClose={() => setRetentionPrompt(null)} label={t("retentionConfirmTitle")}>
        {retentionPrompt && (
          <div class="confirm">
            <h2>{t("retentionConfirmTitle")}</h2>
            <p>
              {retentionPrompt.count === 0
                ? t("retentionConfirmZero", { days: retentionPrompt.days })
                : t(retentionPrompt.count === 1 ? "retentionConfirmOne" : "retentionConfirmMany", {
                    n: retentionPrompt.count,
                    days: retentionPrompt.days,
                  })}
            </p>
            <div class="actions end">
              <button type="button" class="btn" onClick={() => setRetentionPrompt(null)}>
                {t("cancel")}
              </button>
              <button
                type="button"
                class="btn primary"
                onClick={() => {
                  const days = retentionPrompt.days;
                  setRetentionPrompt(null);
                  void setRetention(days);
                }}
              >
                {t("continueAction")}
              </button>
            </div>
          </div>
        )}
      </Dialog>

      <Dialog open={pending !== null} onClose={() => setPending(null)} label={t("deleteConversationTitle")}>
        {pending && (
          <div class="confirm">
            <h2>{t("deleteConversationTitle")}</h2>
            <p>{t("deleteConversationBody", { title: pending.title.trim() || t("untitled") })}</p>
            <div class="actions end">
              <button type="button" class="btn" onClick={() => setPending(null)}>
                {t("cancel")}
              </button>
              <button
                type="button"
                class="btn danger"
                onClick={() => {
                  const id = pending.conversation_id;
                  setPending(null);
                  void deleteConversation(id);
                }}
              >
                {t("delete")}
              </button>
            </div>
          </div>
        )}
      </Dialog>

      <Dialog open={confirming} onClose={() => setConfirming(false)} label={t("deleteAllTitle")}>
        <form
          class="confirm"
          onSubmit={(event) => {
            event.preventDefault();
            if (typed.trim() !== word) return;
            setConfirming(false);
            void deleteAllConversations();
          }}
        >
          <h2>{t("deleteAllTitle")}</h2>
          <p>{t("deleteAllBody")}</p>
          <label class="field-label" for="confirm-word">
            {t("deleteAllType", { word })}
          </label>
          <input
            id="confirm-word"
            type="text"
            autocomplete="off"
            spellcheck={false}
            value={typed}
            autoFocus
            onInput={(event) => setTyped(event.currentTarget.value)}
          />
          <div class="actions end">
            <button type="button" class="btn" onClick={() => setConfirming(false)}>
              {t("cancel")}
            </button>
            <button type="submit" class="btn danger" disabled={typed.trim() !== word}>
              {t("deleteAll")}
            </button>
          </div>
        </form>
      </Dialog>
    </Section>
  );
}

function Setting({
  title,
  help,
  danger = false,
  children,
}: {
  title: string;
  help?: string;
  danger?: boolean;
  children?: ComponentChildren;
}) {
  return (
    <div class="set" data-danger={danger}>
      <div>
        <h3 class="seth">{title}</h3>
        {help && <p class="setp">{help}</p>}
      </div>
      <div class="setcol">{children}</div>
    </div>
  );
}

/**
 * Options that act at once. The checked one always mirrors what the Core
 * reports, so a refused change never looks applied.
 */
function Choice({
  label,
  options,
  current,
  disabled = false,
  onChoose,
}: {
  label: string;
  options: { value: string; text: string }[];
  current: string;
  disabled?: boolean;
  onChoose: (value: string) => void;
}) {
  return (
    <div class="radios" role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          class="radio"
          aria-checked={current === option.value}
          disabled={disabled}
          onClick={() => onChoose(option.value)}
        >
          <span class="radio-mark" aria-hidden="true" />
          <span>{option.text}</span>
        </button>
      ))}
    </div>
  );
}
