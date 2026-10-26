import { reloadModel, t } from "../lib/store";
import type { InferenceView } from "../lib/types";
import { formatTps } from "../lib/present";
import { useStore } from "../lib/store";
import { StatusTag } from "./ui";

/**
 * Explains the model states that stop or slow Donna. A game pause is not
 * here: the global band covers it on every screen.
 */
export function ModelNotice({ inference }: { inference: InferenceView }) {
  const s = useStore();
  if (inference.slow && (inference.state === "ready" || inference.state === "generating")) {
    const tps = formatTps(inference.warmup_tokens_per_second_x10 ?? 0, s.lang);
    return (
      <div class="notice" data-tone="warn" role="status">
        <StatusTag tone="warn">{t("slowTitle")}</StatusTag>
        <p>{t("slowBody", { tps })}</p>
        <div class="actions">
          <button type="button" class="btn" onClick={() => void reloadModel()}>
            {t("reloadModel")}
          </button>
        </div>
      </div>
    );
  }
  switch (inference.state) {
    case "failed":
      return (
        <div class="notice" data-tone="danger" role="alert">
          <StatusTag tone="danger">{t("modelFailedTitle")}</StatusTag>
          <p>{t("modelFailedBody")}</p>
          {inference.last_error && (
            <p class="mono small">
              <span class="t3">{t("lastError")}: </span>
              {inference.last_error}
            </p>
          )}
        </div>
      );
    case "loading":
      return (
        <div class="notice" data-tone="pending" role="status">
          <StatusTag tone="pending">{t("stateLoading")}</StatusTag>
          <p>{t("loadingBody")}</p>
        </div>
      );
    case "disabled":
      return (
        <div class="notice" data-tone="mute">
          <StatusTag tone="mute">{t("stateDisabled")}</StatusTag>
          <p>{t("modelDisabledBody")}</p>
        </div>
      );
    default:
      return null;
  }
}
