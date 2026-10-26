import { useEffect, useRef } from "preact/hooks";
import {
  SCREENS,
  dismissToast,
  getState,
  goTo,
  newConversation,
  setState,
  start,
  stopGenerating,
  t,
  useStore,
} from "../lib/store";
import { CoreScreen } from "./CoreScreen";
import { CloseIcon } from "./Icons";
import { Approvals } from "./screens/Approvals";
import { Chat } from "./screens/Chat";
import { Home } from "./screens/Home";
import { System } from "./screens/System";
import { Work } from "./screens/Work";
import { GlobalBand, Rail, StatusStrip } from "./Shell";

function onKey(event: KeyboardEvent): void {
  const ctrl = event.ctrlKey || event.metaKey;
  const key = event.key.toLowerCase();
  if (ctrl && !event.altKey && /^Digit[1-5]$/.test(event.code)) {
    const screen = SCREENS[Number(event.code.slice(5)) - 1];
    if (screen) {
      event.preventDefault();
      goTo(screen);
    }
  } else if (ctrl && key === "n") {
    event.preventDefault();
    newConversation();
  } else if (ctrl && key === "k") {
    event.preventDefault();
    goTo("chat");
    // The history is a drawer only in narrow windows; otherwise it is always shown.
    if (window.matchMedia("(max-width: 1000px)").matches) setState({ historyOpen: true });
    requestAnimationFrame(() => document.getElementById("search")?.focus());
  } else if (ctrl && event.key === ",") {
    event.preventDefault();
    goTo("system");
  } else if (event.key === "Escape" && !event.defaultPrevented && !document.querySelector("dialog[open]")) {
    const current = getState();
    if (current.historyOpen) {
      setState({ historyOpen: false });
      return;
    }
    // Escape stops a reply only where the reply is on screen.
    if (current.view === "chat" && (document.activeElement as HTMLElement | null)?.id !== "search") {
      void stopGenerating();
    }
  }
}

export function App() {
  const s = useStore();
  const previousView = useRef(s.view);

  useEffect(() => {
    void start();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // After a screen change from the rail, move focus to the new heading so a
  // keyboard or screen reader user lands at the top of the screen.
  useEffect(() => {
    if (previousView.current === s.view) return;
    previousView.current = s.view;
    const active = document.activeElement;
    if (!active || active === document.body || active.closest(".rail")) {
      document.getElementById("screen-title")?.focus({ preventScroll: true });
    }
  }, [s.view]);

  // Before the first answer, and for problems a retry cannot fix while
  // running, the Core screen explains; once connected, a band does.
  const showCoreScreen =
    s.core !== "ok" && (!s.everConnected || s.core === "version_mismatch" || s.core === "config_error");

  return (
    <div class="app">
      <a class="skip" href="#main">
        {t("skipToContent")}
      </a>
      <Rail />
      {/* The room: one sheet of paper on the desk, the ledger line under it. */}
      <div class="room">
        <main id="main" class="sheet" tabIndex={-1}>
          {showCoreScreen ? (
            <CoreScreen />
          ) : (
            <>
              <GlobalBand />
              <div class="screen" data-screen={s.view} key={s.view}>
                {s.view === "home" ? (
                  <Home />
                ) : s.view === "chat" ? (
                  <Chat />
                ) : s.view === "work" ? (
                  <Work />
                ) : s.view === "system" ? (
                  <System />
                ) : (
                  <Approvals />
                )}
              </div>
            </>
          )}
        </main>
        <StatusStrip />
      </div>
      <div class="toasts" role="region" aria-label={t("notifications")}>
        {s.toasts.map((toast) => (
          <div key={toast.id} class="toast" data-tone={toast.tone} role={toast.tone === "error" ? "alert" : "status"}>
            <span>{toast.text}</span>
            <button type="button" class="btn icon" aria-label={t("dismiss")} onClick={() => dismissToast(toast.id)}>
              <CloseIcon />
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
