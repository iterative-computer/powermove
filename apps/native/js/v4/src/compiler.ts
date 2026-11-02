/** Public compiler contract. Apply these aliases before resolving extension imports. */
export const compilerAliases = Object.freeze({
  "powermove/ui": "powermove-v4/ui",
  "powermove/hooks": "powermove-v4/hooks",
  "powermove/jsx-runtime": "powermove-v4/jsx-runtime",
  "powermove/jsx-dev-runtime": "powermove-v4/jsx-dev-runtime",
});
export const compilerOptions = Object.freeze({
  jsx: "react-jsx",
  jsxImportSource: "powermove-v4",
});
