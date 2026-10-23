import { useEffect, useState } from "preact/hooks";
import { toolLabel } from "../lib/i18n";
import { approve, deny, setState, t, useStore } from "../lib/store";
import type { PendingApproval } from "../lib/store";
import { CloseIcon } from "./Icons";
import { Dialog } from "./Dialog";

function formatRemaining(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = String(total % 60).padStart(2, "0");
  return `${minutes}:${seconds}`;
}

function useNow(): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  return now;
}

function Countdown({ deadline, now }: { deadline: number; now: number }) {
  const left = deadline - now;
  return (
    <span class="countdown" data-urgent={left < 30_000}>
      {left <= 0 ? t("expired") : t("expiresIn", { time: formatRemaining(left) })}
    </span>
  );
}

export function ApprovalsDrawer() {
  const s = useStore();
  const close = () => setState({ drawer: false });
  return (
    <Dialog open={s.drawer} onClose={close} label={t("approvalsTitle")} variant="sheet">
      <div class="sheet">
        <header class="sheet-head">
          <h2>{t("approvalsTitle")}</h2>
          <button type="button" class="icon-btn" aria-label={t("close")} onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <p class="muted small">{t("approvalsIntro")}</p>
        {s.approvals.length === 0 ? (
          <div class="sheet-empty">
            <p>{t("approvalsEmpty")}</p>
            <p class="muted small">{t("approvalsEmptyHint")}</p>
          </div>
        ) : (
          <ul class="approvals">
            {s.approvals.map((item) => (
              <li key={item.view.approval_id}>
                <ApprovalCard item={item} />
              </li>
            ))}
          </ul>
        )}
      </div>
    </Dialog>
  );
}

function ApprovalCard({ item }: { item: PendingApproval }) {
  const s = useStore();
  const { view, deadline } = item;
  const [denying, setDenying] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const now = useNow();
  const expired = deadline - now <= 0;
  // Every argument exactly as the Core returned it, never shortened here.
  const lines = Object.entries(view.arguments)
    .map(([key, value]) => `${key}: ${value}`)
    .join("\n");

  const decide = async (action: () => Promise<void>) => {
    setBusy(true);
    await action();
    setBusy(false);
  };

  return (
    <article class="approval" aria-label={toolLabel(s.lang, view.tool)}>
      <div class="approval-head">
        <h3>{toolLabel(s.lang, view.tool)}</h3>
        <Countdown deadline={deadline} now={now} />
      </div>
      <p class="mono small muted tool-raw">{view.tool}</p>
      {view.capabilities.length > 0 && (
        <ul class="caps" aria-label={t("capabilities")}>
          {view.capabilities.map((cap) => (
            <li key={cap} class="mono">
              {cap}
            </li>
          ))}
        </ul>
      )}
      <p class="label">{t("exactArguments")}</p>
      <pre class="args" tabIndex={0}>
        {lines}
      </pre>
      <p class="small muted">
        {t("fingerprint")}: <span class="mono" title={view.fingerprint}>{view.fingerprint.slice(0, 12)}</span>
      </p>
      {denying ? (
        <div class="deny-form">
          <label class="label" for={`reason-${view.approval_id}`}>
            {t("denyReasonLabel")}
          </label>
          <input
            id={`reason-${view.approval_id}`}
            type="text"
            maxLength={200}
            value={reason}
            autoFocus
            onInput={(event) => setReason(event.currentTarget.value)}
          />
          <div class="actions">
            <button type="button" class="btn" onClick={() => setDenying(false)}>
              {t("cancel")}
            </button>
            <button
              type="button"
              class="btn btn-danger"
              disabled={busy}
              onClick={() => void decide(() => deny(view, reason))}
            >
              {t("denyConfirm")}
            </button>
          </div>
        </div>
      ) : (
        <div class="actions">
          <button type="button" class="btn" disabled={busy} onClick={() => setDenying(true)}>
            {t("deny")}
          </button>
          <button
            type="button"
            class="btn btn-primary"
            disabled={busy || expired}
            onClick={() => void decide(() => approve(view))}
          >
            {t("approve")}
          </button>
        </div>
      )}
    </article>
  );
}
