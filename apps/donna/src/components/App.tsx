import { useEffect } from "preact/hooks";
import {
  dismissToast,
  newConversation,
  setState,
  start,
  stopGenerating,
  t,
  useStore,
} from "../lib/store";
import { ApprovalsDrawer } from "./Approvals";
import { Chat } from "./Chat";
import { CoreScreen } from "./CoreScreen";
import { Header } from "./Header";
import { CloseIcon } from "./Icons";
import { Settings } from "./Settings";
import { Sidebar } from "./Sidebar";

export function App() {
  const s = useStore();

  useEffect(() => {
    void start();
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const ctrl = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();
      if (ctrl && key === "n") {
        event.preventDefault();
        newConversation();
      } else if (ctrl && key === "k") {
        event.preventDefault();
        setState({ sidebar: true });
        requestAnimationFrame(() => document.getElementById("search")?.focus());
      } else if (ctrl && event.key === ",") {
        event.preventDefault();
        setState((st) => ({ view: st.view === "settings" ? "chat" : "settings" }));
      } else if (
        event.key === "Escape" &&
        !document.querySelector("dialog[open]") &&
        (document.activeElement as HTMLElement | null)?.id !== "search"
      ) {
        void stopGenerating();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const ready = s.core === "ok";
  return (
    <div class="app" data-sidebar={s.sidebar}>
      <a class="skip" href="#main">
        {t("skipToChat")}
      </a>
      <Sidebar />
      <div class="main-col">
        <Header />
        <main id="main" tabIndex={-1}>
          {!ready ? <CoreScreen /> : s.view === "settings" ? <Settings /> : <Chat />}
        </main>
      </div>
      <ApprovalsDrawer />
      <div class="toasts" role="region" aria-label={t("notifications")}>
        {s.toasts.map((toast) => (
          <div key={toast.id} class="toast" data-tone={toast.tone} role={toast.tone === "error" ? "alert" : "status"}>
            <span>{toast.text}</span>
            <button
              type="button"
              class="icon-btn"
              aria-label={t("dismiss")}
              onClick={() => dismissToast(toast.id)}
            >
              <CloseIcon />
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
