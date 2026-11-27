import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { resolveScheme, translateTheme } from "../index.mjs";

test("scheme and token layer results match actual frozen v1.2.0 function bodies", () => {
  const source = execFileSync(
    "git",
    [
      "show",
      "07ed5fa17ee640659cb075f9cad1af2095011d60:apps/desktop/src/renderer/src/kernel/theme-apply.ts",
    ],
    { cwd: new URL("../../../../../", import.meta.url), encoding: "utf8" },
  );
  const body = (name) => {
    const match = source.match(
      new RegExp(`export function ${name}[^\\{]+\\{([\\s\\S]*?)\\n\\}`),
    );
    assert.ok(match, `Frozen ${name} source shape changed`);
    return match[1];
  };
  // Execute only public frozen function bodies, with synthetic inputs and an
  // injected matchMedia. This never evaluates an installed theme/extension.
  const resolve = Function("mode", "globalThis", body("resolveScheme"));
  const oracleScheme = Function(
    "resolveScheme",
    `return function(def, preference) {${body("themeScheme")}}`,
  );
  const oracleTokens = Function("def", "scheme", body("themeTokens"));
  for (const systemScheme of ["light", "dark"]) {
    const media = { matchMedia: () => ({ matches: systemScheme === "dark" }) };
    for (const scheme of ["auto", "light", "dark"])
      for (const preference of ["system", "light", "dark"]) {
        const input = {
          id: "oracle",
          name: "Oracle",
          scheme,
          tokens: { "--accent": "#123456", "--base": "base" },
          darkTokens: { "--accent": "#abcdef", "--dark": "dark" },
        };
        const expected = oracleScheme((mode) => resolve(mode, media))(
          input,
          preference,
        );
        assert.equal(resolveScheme(input, preference, systemScheme), expected);
        assert.deepEqual(
          translateTheme(input, { foundation: false, preference, systemScheme })
            .inlineTokens,
          oracleTokens(input, expected),
        );
      }
  }
});
