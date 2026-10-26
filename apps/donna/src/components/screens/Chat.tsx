import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import type { StringKey } from "../../lib/i18n";
import { formatClock, formatTps, replyFailureKey } from "../../lib/present";
import {
  deleteConversation,
  exportConversation,
  newConversation,
  openApprovalForJob,
  openConversation,
  regenerate,
  sendMessage,
  setSearch,
  setState,
  submitProposal,
  t,
  useStore,
} from "../../lib/store";
import { relativeTime } from "../../lib/time";
import type { ConversationSummary, MessageView } from "../../lib/types";
import { Composer } from "../Composer";
import { Dialog } from "../Dialog";
import { CloseIcon, DownloadIcon, HistoryIcon, PlusIcon, RefreshIcon, SearchIcon, TrashIcon } from "../Icons";
import { Markdown } from "../Markdown";
import { ModelNotice } from "../ModelNotice";
import { StatusTag } from "../ui";

const SUGGESTIONS: StringKey[] = ["suggestion1", "suggestion2", "suggestion3", "suggestion4"];

const titleOf = (c: ConversationSummary | undefined): string => c?.title.trim() || t("untitled");

/** Below this width (the CSS breakpoint) the history is a drawer over the conversation. */
const DRAWER_QUERY = "(max-width: 1000px)";

function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => window.matchMedia(DRAWER_QUERY).matches);
  useEffect(() => {
    const query = window.matchMedia(DRAWER_QUERY);
    const onChange = () => setNarrow(query.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return narrow;
}

export function Chat() {
  const s = useStore();
  const listRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const shownUserMessage = useRef<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<ConversationSummary | null>(null);
  const historyButton = useRef<HTMLButtonElement>(null);
  const narrow = useNarrow();
  const drawer = narrow && s.historyOpen;
  const wasDrawer = useRef(false);

  // The drawer takes focus when it opens and gives it back to its button
  // when it closes (Escape, Close, the scrim or picking a conversation).
  useEffect(() => {
    if (drawer) {
      requestAnimationFrame(() => document.getElementById("search")?.focus());
    } else if (wasDrawer.current) {
      historyButton.current?.focus();
    }
    wasDrawer.current = drawer;
  }, [drawer]);

  const streaming = s.streaming;
  const stored = streaming ? s.messages.filter((m) => m.message_id !== streaming.turnId) : s.messages;
  const empty = stored.length === 0 && !streaming && !s.loadingConversation;
  const lastAssistant = [...stored].reverse().find((m) => m.role === "assistant");
  const lastMessage = stored[stored.length - 1];
  const canRegenerate =
    !streaming && !s.sending && lastMessage !== undefined && lastMessage.role === "assistant";
  const active = s.conversations.find((c) => c.conversation_id === s.activeId);

  useLayoutEffect(() => {
    const list = listRef.current;
    // A message the person has just sent is brought into view once; while the
    // reply streams below it, scrolling up is left alone.
    if (lastMessage?.role === "user" && lastMessage.message_id !== shownUserMessage.current) {
      shownUserMessage.current = lastMessage.message_id;
      stick.current = true;
    }
    if (!list) return;
    // An empty conversation (the suggestions) starts at the top.
    if (empty) list.scrollTop = 0;
    else if (stick.current) list.scrollTop = list.scrollHeight;
  }, [s.messages, streaming?.text, s.activeId, empty]);

  useEffect(() => {
    stick.current = true;
  }, [s.activeId]);

  const notice =
    s.inference && s.inference.state !== "paused_for_game" ? <ModelNotice inference={s.inference} /> : null;

  return (
    <div class="chat-screen" data-history={s.historyOpen}>
      <History onDelete={setConfirmDelete} />
      <button
        type="button"
        class="scrim"
        tabIndex={-1}
        aria-hidden="true"
        onClick={() => setState({ historyOpen: false })}
      />
      <section class="conv-pane" aria-labelledby="screen-title" inert={drawer}>
        <header class="thead">
          <div class="thead-text">
            <p class="eyebrow">{t("chatEyebrow")}</p>
            <h1 class="tht" id="screen-title" tabIndex={-1}>
              {empty ? t("emptyTitle") : s.activeId ? titleOf(active) : t("newChat")}
            </h1>
            {active && !empty && (
              <p class="thm">
                <span>
                  {t(active.message_count === 1 ? "messageCountOne" : "messageCount", {
                    n: active.message_count,
                  })}
                </span>
                <span>{t("updatedAt", { time: relativeTime(active.updated_at, s.lang) })}</span>
              </p>
            )}
            {empty && <p class="lede">{t("emptyBody")}</p>}
          </div>
          <div class="toolbar">
            <button
              type="button"
              class="btn ghost hist-btn"
              ref={historyButton}
              aria-expanded={drawer}
              aria-controls="history"
              onClick={() => setState({ historyOpen: true })}
            >
              <HistoryIcon />
              {t("openHistory")}
            </button>
            {active && (
              <>
                <button
                  type="button"
                  class="btn ghost"
                  disabled={s.core !== "ok"}
                  onClick={() => void exportConversation(active.conversation_id)}
                >
                  <DownloadIcon />
                  {t("exportMd")}
                </button>
                <button
                  type="button"
                  class="btn ghost"
                  disabled={s.core !== "ok" || streaming !== null}
                  onClick={() => setConfirmDelete(active)}
                >
                  <TrashIcon />
                  {t("delete")}
                </button>
              </>
            )}
          </div>
        </header>
        {notice && <div class="chat-notice">{notice}</div>}
        <div
          class="messages"
          ref={listRef}
          // Focusable so the arrow, Page and Home/End keys scroll the conversation.
          tabIndex={0}
          aria-label={t("messagesLabel")}
          onScroll={(event) => {
            const el = event.currentTarget;
            stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
          }}
        >
          {empty ? (
            <Suggestions />
          ) : (
            <ol class="thread">
              {stored.map((message) => (
                <li key={message.message_id}>
                  <Message message={message} isLastAssistant={canRegenerate && message === lastAssistant} />
                </li>
              ))}
              {streaming && (
                <li key={streaming.turnId}>
                  <StreamingMessage text={streaming.text} />
                </li>
              )}
            </ol>
          )}
        </div>
        <div class="chat-composer">
          <Composer
            id="composer"
            variant="chat"
            value={s.draft}
            onChange={(draft) => setState({ draft })}
            onSend={sendMessage}
            label={t("composerLabel")}
            placeholder={t("composerPlaceholder")}
            hint={t("composerHint")}
          />
          <div class="sr-only" role="status" aria-live="polite">
            {s.announcement}
          </div>
        </div>
      </section>
      <DeleteDialog pending={confirmDelete} onClose={() => setConfirmDelete(null)} />
    </div>
  );
}

function History({ onDelete }: { onDelete: (c: ConversationSummary) => void }) {
  const s = useStore();
  const searching = s.search.trim() !== "";
  return (
    <aside class="hist" id="history" aria-label={t("historyLabel")}>
      <div class="hist-top">
        <button type="button" class="btn new-chat" onClick={newConversation}>
          <PlusIcon />
          <span>{t("newChat")}</span>
          <kbd>Ctrl+N</kbd>
        </button>
        <button
          type="button"
          class="btn icon hist-close"
          aria-label={t("close")}
          onClick={() => setState({ historyOpen: false })}
        >
          <CloseIcon />
        </button>
      </div>
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
          <button type="button" class="btn icon" aria-label={t("clearSearch")} onClick={() => setSearch("")}>
            <CloseIcon />
          </button>
        )}
      </div>
      <nav class="convs" aria-label={t("historyLabel")}>
        {s.conversations.length === 0 ? (
          <p class="hint empty-note">{searching ? t("noResults") : t("noConversations")}</p>
        ) : (
          <ul>
            {s.conversations.map((c) => (
              <li key={c.conversation_id} class="conv-item">
                <button
                  type="button"
                  class="conv"
                  aria-current={c.conversation_id === s.activeId ? "true" : undefined}
                  onClick={() => void openConversation(c.conversation_id)}
                >
                  <span class="convt">{titleOf(c)}</span>
                  {searching && c.snippet ? (
                    <span class="convm">{c.snippet}</span>
                  ) : (
                    <span class="convm">
                      <span>{relativeTime(c.updated_at, s.lang)}</span>
                      <span>
                        {t(c.message_count === 1 ? "messageCountOne" : "messageCount", { n: c.message_count })}
                      </span>
                    </span>
                  )}
                </button>
                <button
                  type="button"
                  class="btn icon conv-delete"
                  aria-label={`${t("deleteConversation")}: ${titleOf(c)}`}
                  onClick={() => onDelete(c)}
                >
                  <TrashIcon />
                </button>
              </li>
            ))}
          </ul>
        )}
      </nav>
    </aside>
  );
}

function DeleteDialog({ pending, onClose }: { pending: ConversationSummary | null; onClose: () => void }) {
  return (
    <Dialog open={pending !== null} onClose={onClose} label={t("deleteConversationTitle")}>
      {pending && (
        <div class="confirm">
          <h2>{t("deleteConversationTitle")}</h2>
          <p>{t("deleteConversationBody", { title: titleOf(pending) })}</p>
          <div class="actions end">
            <button type="button" class="btn" onClick={onClose}>
              {t("cancel")}
            </button>
            <button
              type="button"
              class="btn danger"
              onClick={() => {
                const id = pending.conversation_id;
                onClose();
                void deleteConversation(id);
              }}
            >
              {t("delete")}
            </button>
          </div>
        </div>
      )}
    </Dialog>
  );
}

function Suggestions() {
  const s = useStore();
  const blocked = s.core !== "ok" || s.inference?.state === "paused_for_game" || s.sending;
  return (
    <div class="suggest">
      <p class="eyebrow">{t("suggestionsLabel")}</p>
      <ul class="chips">
        {SUGGESTIONS.map((key) => (
          <li key={key}>
            <button type="button" class="chip" disabled={blocked} onClick={() => void sendMessage(t(key))}>
              {t(key)}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Who({ role, at }: { role: MessageView["role"]; at: string | null }) {
  const s = useStore();
  const time = at ? formatClock(at, s.lang) : "";
  return (
    <p class="who" data-role={role}>
      <span>{role === "user" ? t("you") : t("donna")}</span>
      {time && (
        <time class="time" dateTime={at ?? undefined}>
          {time}
        </time>
      )}
    </p>
  );
}

function Message({ message, isLastAssistant }: { message: MessageView; isLastAssistant: boolean }) {
  const s = useStore();
  if (message.role === "user") {
    return (
      <article class="msg" aria-label={t("you")}>
        <Who role="user" at={message.created_at} />
        <p class="u">{message.content}</p>
      </article>
    );
  }
  const stats = message.stats;
  return (
    <article class="msg" aria-label={t("donna")}>
      <Who role="assistant" at={message.created_at} />
      <div class="d">
        <Markdown text={message.content} />
      </div>
      <Outcome message={message} />
      {message.proposal && message.status === "complete" && (
        <ProposalCard messageId={message.message_id} text={message.proposal} />
      )}
      {(stats || isLastAssistant) && (
        <div class="stats">
          {stats && (
            <span>
              {t("statsLine", {
                tps: formatTps(stats.tokens_per_second_x10, s.lang),
                ms: stats.first_token_ms,
              })}
            </span>
          )}
          {isLastAssistant && (
            <button type="button" class="btn ghost" onClick={() => void regenerate()}>
              <RefreshIcon />
              {t("regenerate")}
            </button>
          )}
        </div>
      )}
    </article>
  );
}

/** A reply that did not complete says so plainly, with the Core's reason when there is one. */
function Outcome({ message }: { message: MessageView }) {
  const s = useStore();
  if (message.status === "complete" || message.status === "streaming") return null;
  if (message.status === "cancelled") {
    return (
      <p class="fail" data-tone="mute">
        <StatusTag tone="mute">{t("statusCancelled")}</StatusTag>
        <span>{t("cancelledNote")}</span>
      </p>
    );
  }
  if (message.status === "interrupted") {
    return (
      <p class="fail" data-tone="warn">
        <StatusTag tone="warn">{t("statusInterrupted")}</StatusTag>
      </p>
    );
  }
  const reason = s.turnErrors[message.message_id];
  const known = reason ? replyFailureKey(reason) : null;
  return (
    <div class="fail" data-tone="danger" role="note">
      <StatusTag tone="danger">{t("statusFailed")}</StatusTag>
      {known && <span>{t(known)}</span>}
      {reason && !known && (
        <span>
          {t("failRaw")} <span class="mono">{reason}</span>
        </span>
      )}
    </div>
  );
}

function StreamingMessage({ text }: { text: string }) {
  return (
    <article class="msg" aria-label={t("donna")} aria-busy="true">
      <Who role="assistant" at={null} />
      {text === "" ? (
        <div class="thinking" role="img" aria-label={t("streamingLabel")}>
          <span />
          <span />
          <span />
        </div>
      ) : (
        <div class="d">
          <Markdown text={text} caret />
        </div>
      )}
    </article>
  );
}

function ProposalCard({ messageId, text }: { messageId: string; text: string }) {
  const s = useStore();
  const proposal = s.proposals[messageId];
  let status: { text: string; tone: "ok" | "danger" | "pending"; open?: boolean } | null = null;
  switch (proposal?.phase) {
    case "submitting":
      status = { text: t("proposalSubmitting"), tone: "pending" };
      break;
    case "waiting":
      status = { text: t("proposalWaiting"), tone: "pending", open: true };
      break;
    case "done":
      status = { text: t("proposalDone"), tone: "ok" };
      break;
    case "failed":
      status = { text: t("proposalFailed"), tone: "danger" };
      break;
    case "cancelled":
      status = { text: t("proposalCancelled"), tone: "danger" };
      break;
    default:
      break;
  }
  return (
    <aside class="proposal paper" aria-label={t("proposalTitle")}>
      <p class="proposal-title">{t("proposalTitle")}</p>
      <pre class="quote" tabIndex={0}>
        {text}
      </pre>
      {status ? (
        <div class="cardf" role="status">
          <StatusTag tone={status.tone}>{status.text}</StatusTag>
          {status.open && (
            <button type="button" class="link" onClick={() => openApprovalForJob(proposal?.jobId)}>
              {t("proposalOpen")}
            </button>
          )}
        </div>
      ) : (
        <div class="cardf">
          <button
            type="button"
            class="btn primary"
            disabled={s.core !== "ok"}
            onClick={() => void submitProposal(messageId, text)}
          >
            {t("proposalAsk")}
          </button>
        </div>
      )}
    </aside>
  );
}
