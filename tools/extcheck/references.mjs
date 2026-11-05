// Build/driver metadata only. Both revisions remain immutable paired inputs.
import { readFileSync } from "node:fs";
export const REFERENCES = Object.freeze(
  JSON.parse(
    readFileSync(new URL("./references.json", import.meta.url), "utf8"),
  ),
);
export function referenceRevision(value = REFERENCES.current) {
  if (!Object.values(REFERENCES).includes(value))
    throw Error("Unreviewed behavior reference");
  return value;
}
