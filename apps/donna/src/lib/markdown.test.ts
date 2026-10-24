import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

function render(source: string, caret = false): HTMLElement {
  const host = document.createElement("div");
  host.appendChild(renderMarkdown(source, { caret }));
  return host;
}

const FORBIDDEN = "script, img, iframe, a, svg, style, object, embed, link, form, input, button";

describe("safe markdown renderer", () => {
  it("renders a script tag as inert text", () => {
    const host = render("<script>alert(1)</script>");
    expect(host.querySelector("script")).toBeNull();
    expect(host.textContent).toBe("<script>alert(1)</script>");
  });

  it("renders img, iframe and event handlers as inert text", () => {
    const source =
      '<img src=x onerror="alert(1)"> <iframe src="//evil"></iframe> <b onclick="x()">hi</b>';
    const host = render(source);
    expect(host.querySelector(FORBIDDEN)).toBeNull();
    expect(host.querySelector("b")).toBeNull();
    expect(host.textContent).toBe(source);
    // The markup is escaped in the serialised DOM, never parsed as tags.
    const serialised = new XMLSerializer().serializeToString(host);
    expect(serialised).not.toMatch(/<img|<iframe|<b /);
    expect(serialised).toContain("&lt;img");
  });

  it("keeps HTML inside code spans and fences as text", () => {
    const host = render("`<img src=x onerror=1>`\n\n```html\n<script>alert(1)</script>\n```");
    expect(host.querySelector("img, script")).toBeNull();
    expect(host.querySelector("code.md-code")?.textContent).toBe("<img src=x onerror=1>");
    expect(host.querySelector("pre code")?.textContent).toBe("<script>alert(1)</script>");
  });

  it("never makes links clickable", () => {
    const host = render("[click](javascript:alert(1)) and https://example.com and <a href='x'>y</a>");
    expect(host.querySelector("a")).toBeNull();
    expect(host.textContent).toContain("[click](javascript:alert(1))");
    expect(host.textContent).toContain("https://example.com");
  });

  it("shows HTML entities literally instead of decoding them", () => {
    const host = render("&lt;b&gt; &amp;");
    expect(host.textContent).toBe("&lt;b&gt; &amp;");
    expect(host.querySelector("b")).toBeNull();
  });

  it("renders the supported subset", () => {
    const host = render(
      "**bold** and *italic* and `code`\nsecond line\n\n- one\n- two\n\n1. a\n2. b",
    );
    expect(host.querySelector("strong")?.textContent).toBe("bold");
    expect(host.querySelector("em")?.textContent).toBe("italic");
    expect(host.querySelector("p code")?.textContent).toBe("code");
    expect(host.querySelector("p br")).not.toBeNull();
    expect(host.querySelectorAll("ul > li")).toHaveLength(2);
    expect(host.querySelectorAll("ol > li")).toHaveLength(2);
  });

  it("does not turn arithmetic asterisks into emphasis", () => {
    const host = render("2 * 3 * 4");
    expect(host.querySelector("em")).toBeNull();
    expect(host.textContent).toBe("2 * 3 * 4");
  });

  it("copes with unfinished markup while streaming", () => {
    const host = render("**bold never closed and `code", true);
    expect(host.querySelector("strong")).toBeNull();
    expect(host.textContent).toContain("**bold never closed and `code");
    expect(host.querySelector(".caret")).not.toBeNull();
    const open = render("```ts\nlet x = 1;", true);
    expect(open.querySelector("pre code")?.textContent).toBe("let x = 1;");
  });

  it("stays fast on pathological emphasis input", () => {
    for (const input of ["*a ".repeat(21000), "**a ".repeat(15000), "`a ".repeat(15000), "_*".repeat(20000)]) {
      const start = performance.now();
      render(input);
      expect(performance.now() - start).toBeLessThan(1000);
    }
  });

  it("builds no element outside the allowed set for hostile input", () => {
    const hostile = [
      "<svg onload=alert(1)>",
      "<style>*{display:none}</style>",
      "<form action=//evil><input name=x></form>",
      "![x](http://evil/p.png)",
      "<details open ontoggle=alert(1)>",
    ].join("\n\n");
    const host = render(hostile);
    const allowed = ["p", "br", "code", "pre", "strong", "em", "ul", "ol", "li"];
    for (const node of host.querySelectorAll("*")) {
      expect(allowed).toContain(node.tagName.toLowerCase());
    }
  });
});
