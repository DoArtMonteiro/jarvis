// A minimal, safe markdown renderer. Model output is untrusted: everything
// is built with createElement/createTextNode, so HTML in the text stays text,
// and links are never made clickable (they stay plain text).
//
// Supported: paragraphs, line breaks, **bold**, *italic*, `inline code`,
// fenced code blocks, bullet and numbered lists. A line starting with # is
// shown as a bold line. Nothing else is interpreted.

const FENCE = /^\s*```(.*)$/;
const BULLET = /^\s*[-*•]\s+(.*)$/;
const NUMBERED = /^\s*\d{1,3}[.)]\s+(.*)$/;
const HEADING = /^\s{0,3}#{1,6}\s+(.*)$/;

const text = (value: string): Text => document.createTextNode(value);

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  return node;
}

/** Append inline markup (code, bold, italic) of `source` to `parent`. */
export function renderInline(source: string, parent: Node): void {
  let buffer = "";
  const flush = () => {
    if (buffer) parent.appendChild(text(buffer));
    buffer = "";
  };
  // A closer scan that finds nothing cannot succeed for any later opener
  // either, so remember the failure instead of scanning again (keeps
  // pathological input such as "*a *a *a ..." linear).
  let noCode = false;
  let noBold = false;
  let noItalic = false;
  let i = 0;
  while (i < source.length) {
    const ch = source.charAt(i);
    if (ch === "`" && !noCode) {
      const end = source.indexOf("`", i + 1);
      if (end === -1) noCode = true;
      if (end > i + 1) {
        flush();
        const code = el("code", "md-code");
        code.textContent = source.slice(i + 1, end);
        parent.appendChild(code);
        i = end + 1;
        continue;
      }
    } else if (ch === "*" && source.startsWith("**", i) && !noBold) {
      const end = source.indexOf("**", i + 2);
      if (end === -1) noBold = true;
      if (end > i + 2 && !/\s/.test(source.charAt(i + 2))) {
        flush();
        const strong = el("strong");
        renderInline(source.slice(i + 2, end), strong);
        parent.appendChild(strong);
        i = end + 2;
        continue;
      }
    } else if (ch === "*" && !noItalic) {
      const next = source.charAt(i + 1);
      if (next && !/\s/.test(next) && next !== "*") {
        let end = source.indexOf("*", i + 1);
        while (end !== -1 && (/\s/.test(source.charAt(end - 1)) || source.charAt(end + 1) === "*")) {
          end = source.indexOf("*", end + 1);
        }
        if (end === -1) noItalic = true;
        if (end > i + 1) {
          flush();
          const em = el("em");
          renderInline(source.slice(i + 1, end), em);
          parent.appendChild(em);
          i = end + 1;
          continue;
        }
      }
    }
    buffer += ch;
    i += 1;
  }
  flush();
}

function renderLines(lines: string[], parent: Node): void {
  lines.forEach((line, index) => {
    if (index > 0) parent.appendChild(el("br"));
    renderInline(line, parent);
  });
}

export interface RenderOptions {
  /** Append a blinking caret after the last text (while streaming). */
  caret?: boolean;
}

/** Render markdown `source` into a fragment of safe DOM nodes. */
export function renderMarkdown(source: string, options: RenderOptions = {}): DocumentFragment {
  const fragment = document.createDocumentFragment();
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  let paragraph: string[] = [];
  let list: { node: HTMLOListElement | HTMLUListElement; ordered: boolean } | null = null;

  const closeParagraph = () => {
    if (paragraph.length === 0) return;
    const p = el("p");
    renderLines(paragraph, p);
    fragment.appendChild(p);
    paragraph = [];
  };

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i] ?? "";
    const fence = FENCE.exec(line);
    if (fence) {
      closeParagraph();
      list = null;
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !FENCE.test(lines[i] ?? "")) {
        body.push(lines[i] ?? "");
        i += 1;
      }
      const pre = el("pre", "md-pre");
      const code = el("code");
      code.textContent = body.join("\n");
      pre.appendChild(code);
      fragment.appendChild(pre);
      continue;
    }
    const bullet = BULLET.exec(line);
    const numbered = bullet ? null : NUMBERED.exec(line);
    const item = bullet ?? numbered;
    if (item) {
      closeParagraph();
      const ordered = numbered !== null;
      if (!list || list.ordered !== ordered) {
        list = { node: ordered ? el("ol") : el("ul"), ordered };
        fragment.appendChild(list.node);
      }
      const li = el("li");
      renderInline(item[1] ?? "", li);
      list.node.appendChild(li);
      continue;
    }
    if (line.trim() === "") {
      closeParagraph();
      list = null;
      continue;
    }
    list = null;
    const heading = HEADING.exec(line);
    if (heading) {
      closeParagraph();
      const p = el("p", "md-heading");
      const strong = el("strong");
      renderInline(heading[1] ?? "", strong);
      p.appendChild(strong);
      fragment.appendChild(p);
      continue;
    }
    paragraph.push(line);
  }
  closeParagraph();

  if (options.caret) {
    const caret = el("span", "caret");
    caret.setAttribute("aria-hidden", "true");
    const last = fragment.lastElementChild;
    if (last && (last.tagName === "P" || last.tagName === "UL" || last.tagName === "OL")) {
      const host = last.tagName === "P" ? last : (last.lastElementChild ?? last);
      host.appendChild(caret);
    } else {
      const p = el("p");
      p.appendChild(caret);
      fragment.appendChild(p);
    }
  }
  return fragment;
}
