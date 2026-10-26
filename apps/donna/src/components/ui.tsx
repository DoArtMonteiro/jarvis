// Small shared building blocks. Semantic state is always an icon plus a
// word, never colour alone.

import type { ComponentChildren } from "preact";
import { useEffect, useState } from "preact/hooks";
import type { Tone } from "../lib/present";
import { AlertIcon, CheckIcon, ClockIcon, ErrorIcon, MinusIcon } from "./Icons";

export function ToneIcon({ tone }: { tone: Tone }) {
  switch (tone) {
    case "ok":
      return <CheckIcon />;
    case "warn":
      return <AlertIcon />;
    case "danger":
      return <ErrorIcon />;
    case "pending":
      return <ClockIcon />;
    case "mute":
      return <MinusIcon />;
  }
}

/** A state word with its icon, coloured by tone. */
export function StatusTag({
  tone,
  children,
  variant = "plain",
}: {
  tone: Tone;
  children: ComponentChildren;
  variant?: "plain" | "pill";
}) {
  return (
    <span class={variant === "pill" ? "tag" : "st"} data-tone={tone}>
      <ToneIcon tone={tone} />
      <span>{children}</span>
    </span>
  );
}

/** A titled section of a screen, separated by a hairline. */
export function Section({
  title,
  count,
  aside,
  id,
  children,
  class: extra,
}: {
  title: string;
  count?: number;
  aside?: ComponentChildren;
  id: string;
  children: ComponentChildren;
  class?: string;
}) {
  return (
    <section class={`sec ${extra ?? ""}`} aria-labelledby={id}>
      <div class="sech">
        <h2 class="sect" id={id}>
          {title}
          {count !== undefined && count > 0 && <span class="n">{count}</span>}
        </h2>
        {aside && <div class="secl">{aside}</div>}
      </div>
      {children}
    </section>
  );
}

/** Screen heading: small label, serif title, optional lede. */
export function ScreenHead({
  eyebrow,
  title,
  lede,
  children,
}: {
  eyebrow: string;
  title: string;
  lede?: ComponentChildren;
  children?: ComponentChildren;
}) {
  return (
    <header class="head">
      <div class="head-text">
        <p class="eyebrow">{eyebrow}</p>
        <h1 class="h1" id="screen-title" tabIndex={-1}>
          {title}
        </h1>
        {lede && <p class="lede">{lede}</p>}
      </div>
      {children}
    </header>
  );
}

/** Label and value in a hairline list. */
export function KV({
  label,
  children,
  tone,
  mono = true,
}: {
  label: string;
  children: ComponentChildren;
  tone?: Tone;
  mono?: boolean;
}) {
  return (
    <div class="kv">
      <dt>{label}</dt>
      <dd class={mono ? "mono" : undefined} data-tone={tone}>
        {tone && <ToneIcon tone={tone} />}
        <span>{children}</span>
      </dd>
    </div>
  );
}

/** The current time, updated every second while mounted. */
export function useNow(): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  return now;
}

/** The brass-ringed monogram. */
export function Monogram({ busy = false }: { busy?: boolean }) {
  return (
    <span class="monogram" data-busy={busy} aria-hidden="true">
      D
    </span>
  );
}
