import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist/", "node_modules/", "src-tauri/"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    rules: {
      // Model text is rendered as text nodes only (src/lib/markdown.ts).
      "no-restricted-properties": [
        "error",
        { property: "innerHTML", message: "Render text with textContent." },
        { property: "outerHTML", message: "Render text with textContent." },
      ],
      "no-restricted-syntax": [
        "error",
        {
          selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']",
          message: "Model text must never be rendered as HTML.",
        },
      ],
    },
  },
);
