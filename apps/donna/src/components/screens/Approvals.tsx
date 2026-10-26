import { useEffect, useRef, useState } from "preact/hooks";
import { toolLabel } from "../../lib/i18n";
import { argumentLines, consequenceKey, formatClock, formatDuration } from "../../lib/present";
import { approve, deny, selectApproval, setState, t, useStore } from "../../lib/store";
import type { Decision, PendingApproval } from "../../lib/store";
import { ScreenHead, StatusTag, ToneIcon, useNow } from "../ui";

export function Approvals() {
  const s = useStore();
  const now = useNow();
  const pending = s.approvals.length;
  // The decided request has left the list; its result stays in its place.
  const selected =
    s.approvals.find((item) => item.view.approval_id === s.selectedApproval) ??
    (s.decision ? undefined : s.approvals[0]);

  return (
    <div class="appr-screen" data-empty={pending === 0}>
      {pending > 0 && (
        // The index of pending requests; the open one is the paper tab that joins the page.
        <aside class="alist" aria-label={t("approvalsListLabel")}>
          <p class="intro" role="status">
            {pending === 1 ? t("approvalsCountOne") : t("approvalsCountMany", { n: pending })}
          </p>
          <ul>
            {s.approvals.map((item) => {
              const left = item.deadline - now;
              return (
                <li key={item.view.approval_id}>
                  <button
                    type="button"
                    class="aitem"
                    aria-current={item === selected ? "true" : undefined}
                    onClick={() => selectApproval(item.view.approval_id)}
                  >
                    <span class="ait">{toolLabel(s.lang, item.view.tool)}</span>
                    <span class="aim">
                      <StatusTag tone={left <= 0 ? "mute" : "pending"}>
                        {left <= 0 ? t("expired") : t("pendingTag")}
                      </StatusTag>
                      {left > 0 && <span class="tn">{formatDuration(left)}</span>}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </aside>
      )}
      <div class="appr-page">
        <ScreenHead eyebrow={t("navApprovals")} title={t("approvalsHeading")} lede={t("approvalsLede")} />
        {pending === 0 ? (
          // Nothing pending: one message, and the last decision if one was just made.
          <div class="appr-empty">
            {s.decision && <Result decision={s.decision} />}
            <p class="dt">{t("approvalsEmptyTitle")}</p>
            <p class="t2">{t("approvalsEmptyHint")}</p>
          </div>
        ) : s.decision && !s.approvals.some((item) => item.view.approval_id === s.decision?.approvalId) ? (
          <Result decision={s.decision} />
        ) : selected ? (
          <Detail key={selected.view.approval_id} item={selected} now={now} />
        ) : (
          <p class="t2">{t("approvalsSelect")}</p>
        )}
      </div>
    </div>
  );
}

function Result({ decision }: { decision: Decision }) {
  const s = useStore();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.focus();
  }, [decision]);
  const tone = decision.outcome === "approved" ? "ok" : decision.outcome === "denied" ? "mute" : "danger";
  const text =
    decision.outcome === "approved"
      ? t("resultApproved")
      : decision.outcome === "denied"
        ? t("resultDenied")
        : `${t("approvalFailed")} ${decision.error ?? ""}`;
  return (
    <div class="result" data-tone={tone} ref={ref} tabIndex={-1}>
      <ToneIcon tone={tone} />
      <div>
        <p class="result-title">{toolLabel(s.lang, decision.tool)}</p>
        <p>{text}</p>
      </div>
    </div>
  );
}

/**
 * The complete request on paper. This is the only place a request can be
 * approved: every argument is shown as the Core reported it.
 */
function Detail({ item, now }: { item: PendingApproval; now: number }) {
  const s = useStore();
  const { view, deadline } = item;
  const [denying, setDenying] = useState(false);
  const reasonInput = useRef<HTMLInputElement>(null);
  const denyButton = useRef<HTMLButtonElement>(null);
  const cancelled = useRef(false);

  // Focus follows the deny form: into the reason when it opens, back to
  // Recusar when it is cancelled.
  useEffect(() => {
    if (denying) {
      reasonInput.current?.focus();
    } else if (cancelled.current) {
      cancelled.current = false;
      denyButton.current?.focus();
    }
  }, [denying]);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const left = deadline - now;
  const expired = left <= 0;
  const consequence = consequenceKey(view.tool, s.lang);
  const job = view.job_id ? s.jobs?.find((j) => j.job_id === view.job_id) : undefined;
  const offline = s.core !== "ok";

  const failed = s.decision?.approvalId === view.approval_id && s.decision.outcome === "failed" ? s.decision : null;

  const run = async (decision: () => Promise<void>) => {
    setState({ decision: null });
    setBusy(true);
    await decision();
    setBusy(false);
  };

  return (
    <article class="detail" aria-labelledby="approval-title">
      <header>
        <h2 class="dt" id="approval-title">
          {toolLabel(s.lang, view.tool)}
        </h2>
        <p class="dsub">
          <span class="mono">{view.tool}</span>
          {" · "}
          {t("requestedAt", { time: formatClock(view.requested_at, s.lang, true) })}
        </p>
      </header>
      <p class="cons">
        {consequence ? t(consequence) : t("consGeneric", { tool: view.tool })}
      </p>
      <div>
        <p class="lbl" id="args-label">
          {t("exactArguments")}
        </p>
        <pre class="args" tabIndex={0} aria-labelledby="args-label">
          {argumentLines(view.arguments)}
        </pre>
        <p class="hint">{t("argsNote")}</p>
      </div>
      <dl class="dl">
        <dt>{view.capabilities.length === 1 ? t("kvCapability") : t("capabilities")}</dt>
        <dd class="mono">{view.capabilities.join(", ")}</dd>
        <dt>{t("fingerprint")}</dt>
        <dd class="mono fp">{view.fingerprint}</dd>
        {job && (
          <>
            <dt>{t("kvOrigin")}</dt>
            <dd>{t("originJob", { goal: job.goal })}</dd>
          </>
        )}
        <dt>{t("kvRequested")}</dt>
        <dd class="mono">{formatClock(view.requested_at, s.lang, true)}</dd>
        <dt>{t("kvExpires")}</dt>
        <dd>
          <StatusTag tone={expired ? "mute" : left < 30_000 ? "warn" : "pending"}>
            {expired ? t("expired") : t("expiresIn", { time: formatDuration(left) })}
          </StatusTag>
        </dd>
        <dt>{t("kvUse")}</dt>
        <dd>{t("useOnce")}</dd>
      </dl>
      {failed && (
        <p class="reason" data-tone="danger" role="alert">
          <ToneIcon tone="danger" />
          <span>
            {t("approvalFailed")} {failed.error}
          </span>
        </p>
      )}
      {expired && (
        <p class="reason" data-tone="warn">
          <ToneIcon tone="warn" />
          <span>{t("expiredNote")}</span>
        </p>
      )}
      {denying ? (
        <form
          class="deny"
          onSubmit={(event) => {
            event.preventDefault();
            void run(() => deny(view, reason));
          }}
        >
          <label class="field-label" for={`reason-${view.approval_id}`}>
            {t("denyReasonLabel")}
          </label>
          <input
            id={`reason-${view.approval_id}`}
            type="text"
            maxLength={200}
            value={reason}
            ref={reasonInput}
            aria-describedby={`reason-help-${view.approval_id}`}
            onInput={(event) => setReason(event.currentTarget.value)}
          />
          <p class="hint" id={`reason-help-${view.approval_id}`}>
            {t("denyReasonHelp")}
          </p>
          <div class="actions">
            <button type="submit" class="btn decide danger" disabled={busy || offline}>
              {t("denyConfirm")}
            </button>
            <button type="button" class="btn decide" disabled={busy}
              onClick={() => {
                cancelled.current = true;
                setDenying(false);
              }}
            >
              {t("cancel")}
            </button>
          </div>
        </form>
      ) : (
        <div class="actions">
          <button
            type="button"
            class="btn decide"
            disabled={busy || expired || offline}
            onClick={() => void run(() => approve(view))}
          >
            {t("approve")}
          </button>
          <button
            type="button"
            class="btn decide danger"
            ref={denyButton}
            disabled={busy || offline}
            onClick={() => setDenying(true)}
          >
            {t("deny")}
          </button>
          <p class="hint">{t("approveHint")}</p>
        </div>
      )}
    </article>
  );
}
