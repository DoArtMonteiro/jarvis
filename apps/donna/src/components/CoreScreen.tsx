import { useEffect, useState } from "preact/hooks";
import { api } from "../lib/api";
import { retryCore, t, useStore } from "../lib/store";
import type { AppInfo } from "../lib/types";
import { Monogram } from "./ui";

/** Shown in place of the chat while the Core is not usable. */
export function CoreScreen() {
  const s = useStore();
  const [info, setInfo] = useState<AppInfo | null>(null);
  useEffect(() => {
    api.appInfo().then(setInfo).catch(() => setInfo(null));
  }, [s.core]);

  const error = s.coreError;
  let title = t("coreUnreachableTitle");
  let body = t("coreUnreachableBody");
  let busy = false;
  switch (s.core) {
    case "connecting":
    case "starting":
      title = t("coreStartingTitle");
      body = t("coreStartingBody");
      busy = true;
      break;
    case "launch_failed":
      title = t("coreLaunchFailedTitle");
      break;
    case "version_mismatch":
      title = t("coreVersionTitle");
      body =
        error?.kind === "version_mismatch"
          ? t("coreVersionBody", { found: error.found, expected: error.expected })
          : t("coreVersionBody", { found: "?", expected: "?" });
      break;
    case "config_error":
      title = t("coreConfigTitle");
      body = t("coreConfigBody");
      break;
    default:
      break;
  }
  const detail =
    error && "message" in error ? error.message : error?.kind === "version_mismatch" ? null : null;
  const configPath = error?.kind === "config" ? error.path : info?.config_path;

  return (
    <section class="core-screen" aria-live="polite">
      <div class="core-card">
        <Monogram busy={busy} />
        <h1 class="h1" id="screen-title" tabIndex={-1}>
          {title}
        </h1>
        <p>{body}</p>
        {busy && <span class="loading-line" aria-hidden="true" />}
        {/* The cause is shown in full, not folded away: it is what the person needs to act. */}
        {(detail || configPath) && (
          <div class="core-reason">
            {detail && (
              <>
                <p class="field-label">{t("technicalDetails")}</p>
                <p class="mono small">{detail}</p>
              </>
            )}
            {configPath && (
              <p class="small">
                <span class="muted">{t("configPath")}: </span>
                <span class="mono">{configPath}</span>
              </p>
            )}
          </div>
        )}
        {!busy && (
          <div class="actions">
            <button type="button" class="btn fill" onClick={() => void retryCore()}>
              {t("retry")}
            </button>
          </div>
        )}
      </div>
    </section>
  );
}
