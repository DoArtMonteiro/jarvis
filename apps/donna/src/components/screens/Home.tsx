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

export function Home() {
  const s = useStore();
  useJobsPolling();
  const now = useNow();
  const date = new Date(now);
  const pending = s.approvals.length;
  const paused = s.inference?.state === "paused_for_game";
  const lede = paused
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
          <div class="sec first">
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
          </div>
          <Section id="home-decisions" title={t("decisionTitle")} count={pending} aside={pending > 0 ? t("decisionHint") : undefined}>
            {pending === 0 ? (
              <p class="empty-line">{t("decisionEmpty")}</p>
            ) : (
              <ul class="list">
                {s.approvals.map((item) => (
                  <li key={item.view.approval_id}>
                    <DecisionRow item={item} now={now} />
                  </li>
                ))}
              </ul>
            )}
          </Section>
          <Section
            id="home-jobs"
            title={t("recentJobsTitle")}
            aside={
              <button type="button" class="link" onClick={() => goTo("work")}>
                {t("seeWork")}
              </button>
            }
          >
            <JobList limit={5} />
          </Section>
        </div>
        <div class="col">
          <Machine />
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
      </div>
    </div>
  );
}

/** A compact, visibly truncated preview. Deciding happens only on the full view. */
function DecisionRow({ item, now }: { item: PendingApproval; now: number }) {
  const s = useStore();
  const { view, deadline } = item;
  const consequence = consequenceKey(view.tool, s.lang);
  const preview = excerpt(approvalExcerptSource(view.arguments), EXCERPT_CHARS);
  const left = deadline - now;
  return (
    <div class="row">
      <div class="row-main">
        <p class="rowt serif">{toolLabel(s.lang, view.tool)}</p>
        {consequence && <p class="rowm">{t(consequence)}</p>}
        {preview.text && (
          <p class="rowm excerpt">
            <span class="lbl">{t("excerptLabel")}</span> <span class="mono">{preview.text}</span>
          </p>
        )}
        <p class="rowm t3">{t("requestedAt", { time: formatClock(view.requested_at, s.lang, true) })}</p>
      </div>
      <div class="right">
        <StatusTag tone={left <= 0 ? "mute" : left < 30_000 ? "warn" : "pending"}>
          {left <= 0 ? t("expired") : t("expiresIn", { time: formatDuration(left) })}
        </StatusTag>
        <button type="button" class="btn" onClick={() => openApproval(view.approval_id)}>
          {t("viewAndDecide")}
        </button>
      </div>
    </div>
  );
}

function Machine() {
  const s = useStore();
  const inference = s.inference;
  const health = s.health;
  const model = inferenceMeta(inference);
  const governor = governorMeta(health?.governor);
  const stats = inference?.last_stats ?? null;
  const unknown = t("unknownValue");
  return (
    <Section
      id="home-machine"
      class="first"
      title={t("machineTitle")}
      aside={
        <button type="button" class="link" onClick={() => goTo("system")}>
          {t("seeSystem")}
        </button>
      }
    >
      <dl class="kvs">
        <KV label={t("modelName")}>{inference?.model ?? unknown}</KV>
        <KV label={t("kvState")} tone={model.tone} mono={false}>
          {t(model.key)}
        </KV>
        <KV label={t("modelProfile")} mono={false}>
          {inference
            ? `${inference.gpu_layers > 0 ? "GPU" : "CPU"} · ${
                inference.profile === "primary" ? t("profilePrimary") : t("profileFallback")
              }`
            : unknown}
        </KV>
        <KV label={t("kvLastReply")}>
          {stats
            ? t("statsLine", { tps: formatTps(stats.tokens_per_second_x10, s.lang), ms: stats.first_token_ms })
            : unknown}
        </KV>
        <KV label={t("contextHeading")}>
          {inference ? t("contextTokens", { n: formatNumber(inference.context_size, s.lang) }) : unknown}
        </KV>
        <KV label={t("kvGovernor")} tone={governor.tone} mono={false}>
          {t(governor.key)}
          {health?.governor.process ? ` · ${health.governor.process}` : ""}
        </KV>
        <KV label={t("kvQueued")}>{health ? formatNumber(health.queued_jobs, s.lang) : unknown}</KV>
        <KV label="Core" tone={s.core === "ok" ? "ok" : "danger"}>
          {s.core !== "ok"
            ? t("coreLost")
            : health
              ? `${t("coreOk")} · ${health.core_version} · ${formatDuration(health.uptime_ms)}`
              : t("coreOk")}
        </KV>
      </dl>
    </Section>
  );
}
