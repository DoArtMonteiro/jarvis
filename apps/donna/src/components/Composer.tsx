import { useLayoutEffect, useRef } from "preact/hooks";
import { composerBlock } from "../lib/present";
import { stopGenerating, t, useStore } from "../lib/store";
import { SendIcon, StopIcon } from "./Icons";
import { ToneIcon } from "./ui";

interface ComposerProps {
  id: string;
  value: string;
  onChange: (value: string) => void;
  /** Resolves false when the text was not sent; the draft is then kept. */
  onSend: (text: string) => Promise<boolean>;
  label: string;
  placeholder: string;
  hint: string;
  variant: "home" | "chat";
  /** Visible heading above the field (Home). */
  title?: string;
}

/** The message field used by Home and Chat. Never swallows a draft. */
export function Composer({ id, value, onChange, onSend, label, placeholder, hint, variant, title }: ComposerProps) {
  const s = useStore();
  const ref = useRef<HTMLTextAreaElement>(null);
  const streaming = s.streaming !== null;
  const busy = streaming || s.sending;
  const block = composerBlock(s.core, s.inference);
  // On Home a reply still streaming elsewhere also blocks a new conversation.
  const reason = block ?? (variant === "home" && busy ? "blockBusy" : null);
  const reasonId = `${id}-reason`;

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || variant !== "chat") return;
    el.style.height = "auto";
    // scrollHeight leaves out the borders; add them back so no scrollbar shows.
    const borders = el.offsetHeight - el.clientHeight;
    el.style.height = `${Math.min(el.scrollHeight + borders, 220)}px`;
  }, [value, variant]);

  const submit = async () => {
    if (busy || block || value.trim() === "") return;
    const text = value;
    onChange("");
    const sent = await onSend(text);
    if (!sent) onChange(text);
  };

  // Beside the field in Chat; on the hint line on Home.
  const action =
    variant === "chat" && streaming ? (
      <button type="button" class="btn btn-stop" onClick={() => void stopGenerating()}>
        <StopIcon />
        <span>{t("stop")}</span>
        <kbd>Esc</kbd>
      </button>
    ) : (
      <button type="submit" class="btn send" disabled={busy || block !== null || value.trim() === ""}>
        <SendIcon />
        <span>{t("send")}</span>
      </button>
    );

  return (
    <form
      class="composer"
      data-variant={variant}
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      {title && (
        <label class="clabel" for={id}>
          {title}
        </label>
      )}
      <div class="cfield">
        <textarea
          id={id}
          ref={ref}
          rows={variant === "home" ? 3 : 1}
          maxLength={6000}
          value={value}
          placeholder={placeholder}
          aria-label={title ? undefined : label}
          aria-describedby={reason ? reasonId : undefined}
          disabled={block !== null}
          onInput={(event) => onChange(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
              event.preventDefault();
              void submit();
            }
          }}
        />
        {variant === "chat" && action}
      </div>
      <div class="cfoot">
        {reason ? (
          <p class="reason" id={reasonId} data-tone={block === "blockOffline" ? "danger" : "warn"}>
            <ToneIcon tone={block === "blockOffline" ? "danger" : "warn"} />
            <span>{t(reason)}</span>
          </p>
        ) : (
          <p class="hint">{hint}</p>
        )}
        {variant === "home" && action}
      </div>
    </form>
  );
}
