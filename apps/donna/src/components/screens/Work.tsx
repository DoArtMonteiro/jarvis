import {
  CAPABILITIES,
  capabilityKey,
  formatClock,
  jobGoalKey,
  jobStatusMeta,
  jobSummaryParts,
} from "../../lib/present";
import { t, useJobsPolling, useStore } from "../../lib/store";
import { relativeTime } from "../../lib/time";
import type { JobView } from "../../lib/types";
import { ScreenHead, Section, StatusTag } from "../ui";

const SOON = ["soonProjects", "soonCalendar", "soonEmail", "soonMemory", "soonVoice", "soonIntegrations"] as const;

export function Work() {
  const s = useStore();
  useJobsPolling();
  return (
    <div class="screen-wrap">
      <ScreenHead eyebrow={t("navWork")} title={t("workTitle")} lede={t("workLede")} />
      <Section id="work-jobs" class="first" title={t("jobsTitle")} count={s.jobs?.length}>
        <JobList limit={20} header />
      </Section>
      <Section id="work-caps" title={t("capsTitle")}>
        <p class="sec-intro">{t("capsIntro")}</p>
        <ul class="tbl caps">
          {CAPABILITIES.map((cap) => {
            const key = capabilityKey(cap.id, s.lang);
            return (
              <li key={cap.id} class="tr">
                <span class="job-goal">
                  <span class="ttl">{key ? t(key) : cap.id}</span>
                  <span class="mono t3 small">{cap.id}</span>
                </span>
                <span class="cap-tags">
                  <span class="tag" data-kind={cap.needsApproval ? "approval" : "none"}>
                    {cap.needsApproval ? t("capNeedsApproval") : t("capNoEffects")}
                  </span>
                  {cap.notYet && <span class="tag" data-kind="soon">{t("capNotYet")}</span>}
                </span>
              </li>
            );
          })}
        </ul>
      </Section>
      <Section id="work-soon" title={t("soonTitle")} class="soon-sec">
        <p class="sec-intro">{t("soonIntro")}</p>
        <ul class="soonlist">
          {SOON.map((key) => (
            <li key={key} class="soonit">
              <span class="soont">{t(key)}</span>
              <span class="soonl">{t("soonTag")}</span>
            </li>
          ))}
        </ul>
      </Section>
    </div>
  );
}

/** Recent jobs as the Core reports them. Shared with Home. */
export function JobList({ limit, header = false }: { limit: number; header?: boolean }) {
  const s = useStore();
  if (s.jobs === null) {
    return <p class="empty-line">{s.jobsError ? t("jobsLoadFailed") : t("loading")}</p>;
  }
  if (s.jobs.length === 0) return <p class="empty-line">{t("jobsEmpty")}</p>;
  return (
    <>
      {s.jobsError && <p class="hint">{t("jobsLoadFailed")}</p>}
      <ul class="tbl jobs">
        {header && (
          <li class="tr hdr" aria-hidden="true">
            <span>{t("jobsColGoal")}</span>
            <span>{t("jobsColStatus")}</span>
            <span>{t("jobsColWhen")}</span>
          </li>
        )}
        {s.jobs.slice(0, limit).map((job) => (
          <li key={job.job_id} class="tr">
            <JobRow job={job} />
          </li>
        ))}
      </ul>
    </>
  );
}

function JobRow({ job }: { job: JobView }) {
  const s = useStore();
  const meta = jobStatusMeta(job.status);
  const changed = job.updated_at !== job.submitted_at;
  const goalKey = jobGoalKey(job.goal);
  const summary = job.summary ? jobSummaryParts(job.summary, s.lang) : null;
  return (
    <>
      <span class="job-goal">
        <span class="ttl">{goalKey ? t(goalKey) : job.goal}</span>
        {summary && <span class="job-note">{summary.join(" · ")}</span>}
        {job.summary && !summary && <span class="job-note mono">{job.summary}</span>}
      </span>
      <StatusTag tone={meta.tone}>{t(meta.key)}</StatusTag>
      <span class="time" title={`${formatClock(job.submitted_at, s.lang, true)}`}>
        {t("jobSubmitted", { time: relativeTime(job.submitted_at, s.lang) })}
        {changed && (
          <>
            <br />
            {t("jobUpdated", { time: relativeTime(job.updated_at, s.lang) })}
          </>
        )}
      </span>
    </>
  );
}
