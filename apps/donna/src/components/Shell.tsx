// The frame around every screen: the rail, the status strip and the global
// band. All of it reads the store; none of it fetches.

import type { ComponentChildren, JSX } from "preact";
import type { StringKey } from "../lib/i18n";
import {
  formatDuration,
  formatNumber,
  formatTps,
  governorMeta,
  holdState,
  inferenceMeta,
} from "../lib/present";
import { type Screen, SCREENS, goTo, retryCore, t, useStore } from "../lib/store";
import { ChatIcon, HomeIcon, LockIcon, ShieldIcon, SystemIcon, WorkIcon } from "./Icons";
import { Monogram, ToneIcon, useNow } from "./ui";

const NAV: Record<Screen, { key: StringKey; icon: () => JSX.Element }> = {
  home: { key: "navHome", icon: HomeIcon },
  chat: { key: "navChat", icon: ChatIcon },
  work: { key: "navWork", icon: WorkIcon },
  system: { key: "navSystem", icon: SystemIcon },
  approvals: { key: "navApprovals", icon: ShieldIcon },
};

export function Rail() {
  const s = useStore();
  // Counts are only shown while the Core can confirm them.
  const live = s.core === "ok";
  const pending = live ? s.approvals.length : 0;
  const queued = live ? (s.health?.queued_jobs ?? 0) : 0;
  return (
    <nav class="rail" aria-label={t("navLabel")}>
      <div class="brand">
        <Monogram />
        <div class="word">
          <div class="wordmark">{t("appName")}</div>
          <div class="wordsub">{t("brandSub")}</div>
        </div>
      </div>
      <ul class="nav">
        {SCREENS.map((screen, index) => {
          const { key, icon: Icon } = NAV[screen];
          const count = screen === "approvals" ? pending : screen === "work" ? queued : 0;
          const countLabel =
            count > 0
              ? t(screen === "approvals" ? "navPendingCount" : "navQueuedCount", { n: count })
              : "";
          return (
            <li key={screen}>
              <button
                type="button"
                class="navb"
                aria-current={s.view === screen ? "page" : undefined}
                aria-keyshortcuts={`Control+${index + 1}`}
                title={`${t(key)} (Ctrl+${index + 1})`}
                onClick={() => goTo(screen)}
              >
                <Icon />
                <span class="navl">
                  {t(key)}
                  {countLabel && <span class="sr-only">, {countLabel}</span>}
                </span>
                {count > 0 && (
                  <span class="navc" data-kind={screen} aria-hidden="true">
                    {count}
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
      <p class="railfoot">{t("railHint")}</p>
    </nav>
  );
}

function Cell({ label, children }: { label: string; children: ComponentChildren }) {
  return (
    <span class="cell">
      <span class="lbl">{label}</span>
      {children}
    </span>
  );
}

/** The ledger line under the sheet: quiet, lowercase, labelled by position. */
export function StatusStrip() {
  const s = useStore();
  const model = inferenceMeta(s.inference);
  const governor = governorMeta(s.health?.governor);
  const ok = s.core === "ok";
  // Nothing the Core cannot confirm right now is shown as a live value.
  const inference = ok ? s.inference : null;
  const stats = inference?.last_stats;
  // The last reply's speed when there is one; otherwise the load-time warm-up, labelled as such.
  const warmup = inference?.warmup_tokens_per_second_x10 ?? null;
  const speed = stats
    ? t("tokensPerSecond", { n: formatTps(stats.tokens_per_second_x10, s.lang) })
    : warmup !== null
      ? t("speedWarmup", { n: formatTps(warmup, s.lang) })
      : t("unknownValue");
  return (
    <div class="ledger" role="group" aria-label={t("stripLabel")}>
      <Cell label={t("stripModel")}>
        <span class="v" data-tone={ok ? model.tone : "mute"}>
          <ToneIcon tone={ok ? model.tone : "mute"} />
          {ok ? t(model.key) : t("unknownValue")}
          {inference && (
            <span class="t3"> · {t("contextTokens", { n: formatNumber(inference.context_size, s.lang) })}</span>
          )}
        </span>
      </Cell>
      <Cell label={t("stripSpeed")}>
        <span class="v tn">{speed}</span>
      </Cell>
      <Cell label={t("stripGovernor")}>
        <span class="v" data-tone={ok ? governor.tone : "mute"}>
          <ToneIcon tone={ok ? governor.tone : "mute"} />
          {ok ? t(governor.key) : t("unknownValue")}
          {ok && s.health?.governor.process && <span> · {s.health.governor.process}</span>}
        </span>
      </Cell>
      <Cell label={t("stripCore")}>
        <span class="v" data-tone={ok ? "ok" : "danger"}>
          <ToneIcon tone={ok ? "ok" : "danger"} />
          {ok ? (s.health ? `${s.health.core_version} · ${t("coreActive")}` : t("coreActive")) : t("coreLost")}
        </span>
      </Cell>
      <span class="local">
        <LockIcon />
        {t("localOnly")}
      </span>
    </div>
  );
}

/** Game pause and lost Core, on every screen. */
export function GlobalBand() {
  const s = useStore();
  const now = useNow();
  if (s.core !== "ok") {
    const reconnecting = s.core === "connecting" || s.core === "starting";
    return (
      <div class="band" data-tone="danger" role="alert">
        <ToneIcon tone="danger" />
        <div>
          <h2>{t("bandOfflineTitle")}</h2>
          <p>{t("bandOfflineBody")}</p>
        </div>
        <div class="aside">
          {reconnecting ? (
            <span class="counter">{t("bandReconnecting")}</span>
          ) : (
            <button type="button" class="btn" onClick={() => void retryCore()}>
              {t("retry")}
            </button>
          )}
        </div>
      </div>
    );
  }
  const hold = holdState(s.health, s.inference);
  if (!hold) return null;
  const titles = {
    paused: "pausedTitle",
    resuming: "bandResumingTitle",
    fallback: "bandFallbackTitle",
    detection: "bandDetectionTitle",
  } as const;
  const body =
    hold.kind === "paused"
      ? hold.process
        ? t("pausedBodyProcess", { process: hold.process })
        : t("pausedBody")
      : hold.kind === "resuming"
        ? t("bandResumingBody")
        : hold.kind === "fallback"
          ? t("bandFallbackBody")
          : t("bandDetectionBody");
  const held = hold.heldMs !== null ? formatDuration(hold.heldMs + Math.max(0, now - s.healthAt)) : null;
  return (
    <div class="band" data-tone="warn">
      <ToneIcon tone="warn" />
      {/* Announced once; the live counter beside it is not a live region. */}
      <div role="status">
        <h2>{t(titles[hold.kind])}</h2>
        <p>
          {body}
          {hold.overdue && ` ${t("overdueNote")}`}
        </p>
      </div>
      <div class="aside">
        {hold.process && <span class="counter">{hold.process}</span>}
        {held && <span class="counter">{t("heldFor", { time: held })}</span>}
      </div>
    </div>
  );
}
