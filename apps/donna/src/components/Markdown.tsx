import { useLayoutEffect, useRef } from "preact/hooks";
import { renderMarkdown } from "../lib/markdown";

/** Untrusted text rendered as inert DOM nodes (never as HTML). */
export function Markdown({ text, caret = false }: { text: string; caret?: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    ref.current?.replaceChildren(renderMarkdown(text, { caret }));
  }, [text, caret]);
  return <div class="md" ref={ref} />;
}
