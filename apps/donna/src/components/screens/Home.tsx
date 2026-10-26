import { toolLabel } from "../../lib/i18n";
import {
  approvalExcerptSource,
  consequenceKey,
  excerpt,
  formatClock,
  formatDuration,
  formatNumber,
  formatTps,
  governorMeta,
  greetingKey,
  inferenceMeta,
} from "../../lib/present";
import {
  goTo,
  openApproval,
  openConversation,
  setState,
  startConversation,
  t,
  useStore,
  useJobsPolling,
} from "../../lib/store";
import type { PendingApproval } from "../../lib/store";
import { relativeTime } from "../../lib/time";
import { Composer } from "../Composer";
import { JobList } from "./Work";
import { KV, ScreenHead, Section, StatusTag, useNow } from "../ui";

const EXCERPT_CHARS = 90;
/** Home shows at most this many requests; the rest wait on Approvals. */
const SLIPS = 2;

export function Home() {
  const s = useStore();
  useJobsPolling();
  const now = useNow();
  const date = new Date(now);
  // Requests are only shown while the Core can confirm they are still pending.
  const approvals = s.core === "ok" ? s.approvals : [];
  const pending = approvals.length;
  const paused = s.inference?.state === "paused_for_game";
  const lede =
    s.core !== "ok"
      ? t("homeLedeOffline")
      : paused
        ? t("homeLedeGame")
        : pending === 0
          ? t("homeLedeNone")
          : pending === 1
            ? t("homeLedeOne")
            : t("homeLedeMany", { n: pending });
  const eyebrow = new Intl.DateTimeFormat(s.lang === "pt" ? "pt-PT" : "en-GB", {
    weekday: "long",
    day: "numeric",
    month: "long",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(date);

  return (
    <div class="screen-wrap">
      <ScreenHead eyebrow={eyebrow} title={t(greetingKey(date.getHours()))} lede={lede} />
      <div class="two">
        <div class="col">
          {pending > 0 && (
            <div class="slips">
              {approvals.slice(0, SLIPS).map((item) => (
                <DecisionSlip key={item.view.approval_id} item={item} now={now} />
              ))}
              {pending > SLIPS && (
                <button type="button" class="link" onClick={() => goTo("approvals")}>
                  {t("moreDecisions", { n: pending - SLIPS })}
                </button>
              )}
            </div>
          )}
          <Composer
            id="home-composer"
            variant="home"
            title={t("homeComposerTitle")}
            value={s.homeDraft}
            onChange={(homeDraft) => setState({ homeDraft })}
            onSend={startConversation}
            label={t("composerLabel")}
            placeholder={t("homeComposerPlaceholder")}
            hint={t("homeComposerHint")}
          />
          <Section
            id="home-convs"
            title={t("recentConvsTitle")}
            aside={
              <button type="button" class="link" onClick={() => goTo("chat")}>
                {t("seeAll")}
              </button>
            }
          >
            {s.conversations.length === 0 ? (
              <p class="empty-line">{t("noConversations")}</p>
            ) : (
              <ul class="list">
                {s.conversations.slice(0, 5).map((c) => (
                  <li key={c.conversation_id}>
                    <button type="button" class="rowb" onClick={() => void openConversation(c.conversation_id)}>
                      <span class="rowt">{c.title.trim() || t("untitled")}</span>
                      <span class="rowm">
                        {relativeTime(c.updated_at, s.lang)} ·{" "}
                        {t(c.message_count === 1 ? "messageCountOne" : "messageCount", { n: c.message_count })}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Section>
        </div>
        <div class="col">
          <Machine />
          <Section
            id="home-jobs"
            title={t("recentJobsTitle")}
            aside={
              <button type="button" class="link" onClick={() => goTo("work")}>
                {t("seeWork")}
              </button>
            }
          >
            <JobList limit={4} />
          </Section>
        </div>
      </div>
    </div>
  );
}

/** A compact, visibly truncated preview on a paper slip. Deciding happens only on the full view. */
function DecisionSlip({ item, now }: { item: PendingApproval; now: number }) {
  const s = useStore();
  const { view, deadline } = item;
  const consequence = consequenceKey(view.tool, s.lang);
  const preview = excerpt(approvalExcerptSource(view.arguments), EXCERPT_CHARS);
  const left = deadline - now;
  const titleId = `slip-${view.approval_id}`;
  return (
    <article class="slip" aria-labelledby={titleId}>
      <div class="slip-head">
        <span class="cap">{t("decisionTitle")}</span>
        <span class="hint">{t("decisionHint")}</span>
      </div>
      <h2 class="slip-t" id={titleId}>
        {toolLabel(s.lang, view.tool)}
      </h2>
      {consequence && <p class="slip-cons">{t(consequence)}</p>}
      {preview.text && (
        <p class="excerpt">
          <span class="sr-only">{t("excerptLabel")}: </span>
          <span class="mono">{preview.text}</span>
        </p>
      )}
      <div class="slip-foot">
        <div class="slip-meta">
          <StatusTag tone={left <= 0 ? "mute" : left < 30_000 ? "warn" : "pending"}>
            {left <= 0 ? t("expired") : t("expiresIn", { time: formatDuration(left) })}
          </StatusTag>
          <span class="tn">{t("requestedAt", { time: formatClock(view.requested_at, s.lang, true) })}</span>
        </div>
        <button type="button" class="btn" onClick={() => openApproval(view.approval_id)}>
          {t("viewAndDecide")}
        </button>
      </div>
    </article>
  );
}

/** Machine facts on the dark desk pad, apart from the reading surface. */
function Machine() {
  const s = useStore();
  const offline = s.core !== "ok";
  const inference = s.inference;
  const health = s.health;
  const model = inferenceMeta(inference);
  const governor = governorMeta(health?.governor);
  // A speed is only current while a model is loaded.
  const loaded = inference?.state === "ready" || inference?.state === "generating";
  const stats = loaded ? (inference?.last_stats ?? null) : null;
  const warmup = loaded ? (inference?.warmup_tokens_per_second_x10 ?? null) : null;
  const unknown = t("unknownValue");
  // The figure says which measurement it is: the last reply, or the warm-up on load.
  const speed = stats ? stats.tokens_per_second_x10 : warmup;
  const speedLabel = !loaded
    ? t("speedUnloaded")
    : stats
      ? t("speedLastReply")
      : warmup !== null
        ? t("speedOnLoad")
        : t("speedNone");
  return (
    <section class="deskpad" aria-labelledby="home-machine">
      <div class="pad-head">
        <h2 class="pad-t" id="home-machine">
          {t("machineTitle")}
        </h2>
        <button type="button" class="link" onClick={() => goTo("system")}>
          {t("seeSystem")}
        </button>
      </div>
      {offline ? (
        <p class="pad-note">{t("machineOffline")}</p>
      ) : (
        <>
          <div class="figures">
            <p class="figure">
              <span class="fl">{speedLabel}</span>
              <span class="fv">
                {speed !== null ? formatTps(speed, s.lang) : unknown}
                {speed !== null && <span class="fu"> tok/s</span>}
              </span>
            </p>
            <p class="figure">
              <span class="fl">{t("contextShort")}</span>
              <span class="fv">
                {inference ? formatNumber(inference.context_size, s.lang) : unknown}
                {inference && <span class="fu"> tokens</span>}
              </span>
            </p>
          </div>
          <dl class="kvs">
            <KV label={t("modelName")} wide>
              {inference?.model ?? unknown}
            </KV>
            <KV label={t("kvState")} tone={model.tone}>
              {t(model.key)}
            </KV>
            <KV label={t("modelProfile")}>
              {inference
                ? `${inference.gpu_layers > 0 ? "GPU" : "CPU"} · ${
                    inference.profile === "primary" ? t("profilePrimary") : t("profileFallback")
                  }`
                : unknown}
            </KV>
            <KV label={t("kvGovernor")} tone={governor.tone}>
              {t(governor.key)}
              {health?.governor.process ? ` · ${health.governor.process}` : ""}
            </KV>
            <KV label={t("kvQueued")}>{health ? formatNumber(health.queued_jobs, s.lang) : unknown}</KV>
            <KV label="Core" tone="ok" wide>
              {health ? `${health.core_version} · ${t("coreActive")} · ${formatDuration(health.uptime_ms)}` : t("coreActive")}
            </KV>
          </dl>
        </>
      )}
      <p class="pad-foot">{t("padFoot")}</p>
    </section>
  );
}
