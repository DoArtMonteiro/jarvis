import { useState } from "preact/hooks";
import {
  deleteConversation,
  newConversation,
  openConversation,
  setSearch,
  t,
  useStore,
} from "../lib/store";
import { relativeTime } from "../lib/time";
import { Dialog } from "./Dialog";
import { CloseIcon, PlusIcon, SearchIcon, TrashIcon } from "./Icons";
import type { ConversationSummary } from "../lib/types";

export function Sidebar() {
  const s = useStore();
  const [pending, setPending] = useState<ConversationSummary | null>(null);
  const searching = s.search.trim() !== "";

  return (
    <aside class="sidebar" aria-label={t("historyLabel")} data-open={s.sidebar}>
      <div class="brand">
        <span class="wordmark">{t("appName")}</span>
      </div>
      <button type="button" class="btn btn-primary new-chat" onClick={newConversation}>
        <PlusIcon />
        <span>{t("newChat")}</span>
        <kbd>Ctrl+N</kbd>
      </button>
      <div class="search">
        <SearchIcon />
        <input
          id="search"
          type="search"
          value={s.search}
          placeholder={t("searchPlaceholder")}
          aria-label={t("searchLabel")}
          onInput={(event) => setSearch(event.currentTarget.value)}
        />
        {searching && (
          <button
            type="button"
            class="icon-btn"
            aria-label={t("clearSearch")}
            onClick={() => setSearch("")}
          >
            <CloseIcon />
          </button>
        )}
      </div>
      <nav class="history">
        {s.conversations.length === 0 ? (
          <p class="muted empty-note">{searching ? t("noResults") : t("noConversations")}</p>
        ) : (
          <ul>
            {s.conversations.map((c) => (
              <li key={c.conversation_id} class="conv" data-active={c.conversation_id === s.activeId}>
                <button
                  type="button"
                  class="conv-open"
                  aria-current={c.conversation_id === s.activeId ? "true" : undefined}
                  onClick={() => void openConversation(c.conversation_id)}
                >
                  <span class="conv-title">{c.title.trim() || t("untitled")}</span>
                  {searching && c.snippet ? (
                    <span class="conv-snippet">{c.snippet}</span>
                  ) : (
                    <span class="conv-meta">
                      {[relativeTime(c.updated_at, s.lang), t(c.message_count === 1 ? "messageCountOne" : "messageCount", { n: c.message_count })]
                        .filter(Boolean)
                        .join(" · ")}
                    </span>
                  )}
                </button>
                <button
                  type="button"
                  class="icon-btn conv-delete"
                  aria-label={`${t("deleteConversation")}: ${c.title.trim() || t("untitled")}`}
                  onClick={() => setPending(c)}
                >
                  <TrashIcon />
                </button>
              </li>
            ))}
          </ul>
        )}
      </nav>
      <Dialog open={pending !== null} onClose={() => setPending(null)} label={t("deleteConversationTitle")}>
        {pending && (
          <div class="confirm">
            <h2>{t("deleteConversationTitle")}</h2>
            <p>{t("deleteConversationBody", { title: pending.title.trim() || t("untitled") })}</p>
            <div class="actions">
              <button type="button" class="btn" onClick={() => setPending(null)}>
                {t("cancel")}
              </button>
              <button
                type="button"
                class="btn btn-danger"
                onClick={() => {
                  const id = pending.conversation_id;
                  setPending(null);
                  void deleteConversation(id);
                }}
              >
                {t("delete")}
              </button>
            </div>
          </div>
        )}
      </Dialog>
    </aside>
  );
}
