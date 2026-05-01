import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    "out/**",
    "build/**",
    "dist/**",
    "next-env.d.ts",
  ]),
  {
    files: ["src/**/*.{ts,tsx}"],
    rules: {
      // Block arbitrary Tailwind text-size values (`text-[12px]`, `text-[9.5px]`,
      // etc.). Use the semantic typography utilities defined in globals.css
      // (`text-ui`, `text-meta`, `text-label`, `text-tag`, `text-code`,
      // `text-disasm`, `text-section-h1`) so density tweaks land in one place
      // and the type scale doesn't fragment again. If a new font-size is
      // genuinely needed, add a token first; then this rule allows it.
      "no-restricted-syntax": [
        "error",
        {
          selector: "Literal[value=/text-\\[\\d+(?:\\.\\d+)?px\\]/]",
          message: "Use a semantic typography utility (text-ui, text-meta, text-code, text-disasm, text-label, text-tag, text-section-h1) instead of an arbitrary text-[Npx] value. Add a new --text-* token in globals.css if needed.",
        },
        {
          selector: "TemplateElement[value.cooked=/text-\\[\\d+(?:\\.\\d+)?px\\]/]",
          message: "Use a semantic typography utility (text-ui, text-meta, text-code, text-disasm, text-label, text-tag, text-section-h1) instead of an arbitrary text-[Npx] value. Add a new --text-* token in globals.css if needed.",
        },
      ],
    },
  },
]);

export default eslintConfig;
