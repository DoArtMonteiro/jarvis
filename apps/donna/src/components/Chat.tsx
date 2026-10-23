import { useEffect, useLayoutEffect, useRef } from "preact/hooks";
import type { StringKey } from "../lib/i18n";
import {
  regenerate,
  sendMessage,
  setState,
  stopGenerating,
  submitProposal,
  t,
  useStore,
} from "../lib/store";
import type { MessageView } from "../lib/types";
import { ModelNotice } from "./Header";
import { RefreshIcon, SendIcon, StopIcon } from "./Icons";
import { Markdown } from "./Markdown";

const SUGGESTIONS: StringKey[] = ["suggestion1", "suggestion2", "suggestion3", "suggestion4"];

export function Chat() {
  const s = useStore();
  const listRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  const streaming = s.streaming;
  const stored = streaming ? s.messages.filter((m) => m.message_id !== streaming.turnId) : s.messages;
  const empty = stored.length === 0 && !streaming && !s.loadingConversation;
  const lastAssistant = [...stored].reverse().find((m) => m.role === "assistant");
  const lastMessage = stored[stored.length - 1];
  const canRegenerate =
    !streaming && !s.sending && lastMessage !== undefined && lastMessage.role === "assistant";

  useLayoutEffect(() => {
    const list = listRef.current;
    // A message the person has just sent is always brought into view.
    if (lastMessage?.role === "user") stick.current = true;
    if (list && stick.current) list.scrollTop = list.scrollHeight;
  }, [s.messages, streaming?.text, s.activeId]);

  useEffect(() => {
    stick.current = true;
  }, [s.activeId]);

  return (
    <section class="chat" aria-label={t("messagesLabel")}>
      {s.inference && s.inference.state !== "ready" && s.inference.state !== "generating" && (
        <div class="chat-notice">
          <ModelNotice inference={s.inference} />
        </div>
      )}
      <div
        class="messages"
        ref={listRef}
        onScroll={(event) => {
          const el = event.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
        }}
      >
        {empty ? (
          <EmptyState />
        ) : (
          <ol class="thread">
            {stored.map((message) => (
              <li key={message.message_id}>
                <Message
                  message={message}
                  isLastAssistant={canRegenerate && message === lastAssistant}
                />
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
      <Composer canRegenerate={canRegenerate} />
    </section>
  );
}

function EmptyState() {
  const s = useStore();
  return (
    <div class="empty">
      <h1 class="display">{t("emptyTitle")}</h1>
      <p>{t("emptyBody")}</p>
      <ul class="chips" aria-label={t("suggestionsLabel")}>
        {SUGGESTIONS.map((key) => (
          <li key={key}>
            <button
              type="button"
              class="chip"
              disabled={s.core !== "ok"}
              onClick={() => void sendMessage(t(key))}
            >
              {t(key)}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Message({ message, isLastAssistant }: { message: MessageView; isLastAssistant: boolean }) {
  if (message.role === "user") {
    return (
      <article class="msg msg-user" aria-label={t("you")}>
        <div class="bubble">
          <p class="plain">{message.content}</p>
        </div>
      </article>
    );
  }
  const stats = message.stats;
  const note: Partial<Record<MessageView["status"], StringKey>> = {
    cancelled: "statusCancelled",
    failed: "statusFailed",
    interrupted: "statusInterrupted",
  };
  const noteKey = note[message.status];
  return (
    <article class="msg msg-donna" aria-label={t("donna")}>
      <div class="author">{t("donna")}</div>
      <Markdown text={message.content} />
      {noteKey && <p class="msg-note">{t(noteKey)}</p>}
      {message.proposal && <ProposalCard messageId={message.message_id} text={message.proposal} />}
      <div class="msg-foot">
        {stats && (
          <span class="stats">
            {t("statsLine", {
              tps: (stats.tokens_per_second_x10 / 10).toFixed(1),
              ms: stats.first_token_ms,
            })}
          </span>
        )}
        {isLastAssistant && (
          <button type="button" class="btn btn-ghost" onClick={() => void regenerate()}>
            <RefreshIcon />
            {t("regenerate")}
          </button>
        )}
      </div>
    </article>
  );
}

function StreamingMessage({ text }: { text: string }) {
  return (
    <article class="msg msg-donna streaming" aria-label={t("donna")} aria-busy="true">
      <div class="author">{t("donna")}</div>
      {text === "" ? (
        <div class="thinking" role="img" aria-label={t("streamingLabel")}>
          <span />
          <span />
          <span />
        </div>
      ) : (
        <Markdown text={text} caret />
      )}
    </article>
  );
}

function ProposalCard({ messageId, text }: { messageId: string; text: string }) {
  const s = useStore();
  const proposal = s.proposals[messageId];
  let status: { text: string; tone: string; hint?: string } | null = null;
  switch (proposal?.phase) {
    case "submitting":
      status = { text: t("proposalSubmitting"), tone: "busy" };
      break;
    case "waiting":
      status = { text: t("proposalWaiting"), tone: "wait", hint: t("proposalWaitingHint") };
      break;
    case "done":
      status = { text: t("proposalDone"), tone: "ok" };
      break;
    case "failed":
      status = { text: t("proposalFailed"), tone: "error" };
      break;
    case "cancelled":
      status = { text: t("proposalCancelled"), tone: "error" };
      break;
    default:
      break;
  }
  return (
    <aside class="proposal" aria-label={t("proposalTitle")}>
      <p class="proposal-title">{t("proposalTitle")}</p>
      <pre class="proposal-text">{text}</pre>
      {status ? (
        <div class="proposal-status" data-tone={status.tone} role="status">
          <span>{status.text}</span>
          {status.hint && (
            <button type="button" class="btn btn-ghost" onClick={() => setState({ drawer: true })}>
              {t("proposalOpen")}
            </button>
          )}
        </div>
      ) : (
        <button
          type="button"
          class="btn btn-primary"
          disabled={s.core !== "ok"}
          onClick={() => void submitProposal(messageId, text)}
        >
          {t("proposalAsk")}
        </button>
      )}
    </aside>
  );
}

function Composer({ canRegenerate }: { canRegenerate: boolean }) {
  const s = useStore();
  const draft = s.draft;
  const setDraft = (value: string) => setState({ draft: value });
  const ref = useRef<HTMLTextAreaElement>(null);
  const busy = s.streaming !== null || s.sending;
  const offline = s.core !== "ok";

  const resize = () => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 220)}px`;
  };
  useLayoutEffect(resize, [draft]);

  const submit = async () => {
    if (busy || offline || draft.trim() === "") return;
    const text = draft;
    setDraft("");
    const sent = await sendMessage(text);
    if (!sent) setDraft(text);
  };

  return (
    <div class="composer-wrap">
      <form
        class="composer"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <textarea
          id="composer"
          ref={ref}
          rows={1}
          maxLength={6000}
          value={draft}
          placeholder={t("composerPlaceholder")}
          aria-label={t("composerLabel")}
          onInput={(event) => setDraft(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
              event.preventDefault();
              void submit();
            }
          }}
        />
        {s.streaming ? (
          <button type="button" class="btn btn-stop" onClick={() => void stopGenerating()}>
            <StopIcon />
            <span>{t("stop")}</span>
          </button>
        ) : (
          <button
            type="submit"
            class="btn btn-primary btn-send"
            disabled={busy || offline || draft.trim() === ""}
            aria-label={t("send")}
          >
            <SendIcon />
            <span>{t("send")}</span>
          </button>
        )}
      </form>
      <p class="hint muted small" data-regen={canRegenerate}>
        {t("composerHint")}
      </p>
      <div class="sr-only" role="status" aria-live="polite">
        {s.announcement}
      </div>
    </div>
  );
}
