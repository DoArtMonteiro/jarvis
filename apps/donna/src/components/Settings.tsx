import { useEffect, useState } from "preact/hooks";
import { api } from "../lib/api";
import type { LangSetting } from "../lib/i18n";
import {
  deleteAllConversations,
  setContextSize,
  setLanguage,
  setState,
  t,
  useStore,
} from "../lib/store";
import { Dialog } from "./Dialog";
import { ArrowLeftIcon } from "./Icons";

const CONTEXT_SIZES = [4096, 8192, 16384, 32768];

export function Settings() {
  const s = useStore();
  const [confirming, setConfirming] = useState(false);
  const [typed, setTyped] = useState("");
  const [appVersion, setAppVersion] = useState("—");
  useEffect(() => {
    api
      .appInfo()
      .then((info) => setAppVersion(info.app_version))
      .catch(() => {});
  }, []);
  const word = t("deleteAllWord");
  const languages: [LangSetting, string][] = [
    ["pt_pt", t("langPt")],
    ["en", t("langEn")],
    ["auto", t("langAuto")],
  ];

  return (
    <section class="settings" aria-labelledby="settings-title">
      <div class="settings-inner">
        <header class="settings-head">
          <button
            type="button"
            class="icon-btn"
            aria-label={t("close")}
            onClick={() => setState({ view: "chat" })}
          >
            <ArrowLeftIcon />
          </button>
          <h1 id="settings-title" class="display">
            {t("settingsTitle")}
          </h1>
        </header>

        <section class="card">
          <h2>{t("languageHeading")}</h2>
          <p class="muted">{t("languageHelp")}</p>
          <div class="segmented" role="radiogroup" aria-label={t("languageHeading")}>
            {languages.map(([value, label]) => (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={s.setting === value}
                onClick={() => setLanguage(value)}
              >
                {label}
              </button>
            ))}
          </div>
        </section>

        <section class="card">
          <h2>{t("contextHeading")}</h2>
          <p class="muted">{t("contextHelp")}</p>
          <div class="segmented" role="radiogroup" aria-label={t("contextHeading")}>
            {CONTEXT_SIZES.map((size) => (
              <button
                key={size}
                type="button"
                role="radio"
                aria-checked={s.inference?.context_size === size}
                disabled={!s.inference || s.inference.state === "disabled" || s.core !== "ok"}
                onClick={() => void setContextSize(size)}
              >
                {size.toLocaleString(s.lang === "pt" ? "pt-PT" : "en-GB")}
              </button>
            ))}
          </div>
          {s.contextError && (
            <p class="inline-error" role="alert">
              {s.contextError}
            </p>
          )}
        </section>

        <section class="card">
          <h2>{t("storageHeading")}</h2>
          <p>{t("storageBody")}</p>
        </section>

        <section class="card">
          <h2>{t("clipboardHeading")}</h2>
          <p>{t("clipboardBody")}</p>
        </section>

        <section class="card card-danger">
          <h2>{t("dangerHeading")}</h2>
          <button
            type="button"
            class="btn btn-danger"
            disabled={s.core !== "ok"}
            onClick={() => {
              setTyped("");
              setConfirming(true);
            }}
          >
            {t("deleteAll")}
          </button>
        </section>

        <p class="muted small about">
          {t("aboutVersion", { app: appVersion, core: s.health?.core_version ?? "—" })}
        </p>
      </div>

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
          <label class="label" for="confirm-word">
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
          <div class="actions">
            <button type="button" class="btn" onClick={() => setConfirming(false)}>
              {t("cancel")}
            </button>
            <button type="submit" class="btn btn-danger" disabled={typed.trim() !== word}>
              {t("deleteAll")}
            </button>
          </div>
        </form>
      </Dialog>
    </section>
  );
}
