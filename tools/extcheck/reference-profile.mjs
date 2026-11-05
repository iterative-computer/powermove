import { cp, mkdir, writeFile } from "node:fs/promises";
import path from "node:path";

/** Disposable cold profile only. The shipping marker forces Store isolation
 * even without an account or provenance database; it never grants full access.
 */
export async function prepareReferenceExtension(profile, source, id, store) {
  if (!/^[a-z0-9][a-z0-9._-]*$/.test(id)) throw Error("Unsafe extension id");
  if (
    store &&
    ![store.repoId, store.releaseId].every(
      (value) => typeof value === "string" && /^[a-zA-Z0-9_-]+$/.test(value),
    )
  )
    throw Error("Missing verified Store identity");
  const directory = path.join(profile, "extensions", id);
  await mkdir(path.dirname(directory), { recursive: true });
  await cp(source, directory, { recursive: true });
  if (store)
    await writeFile(
      path.join(directory, ".powermove-store.json"),
      JSON.stringify({ repoId: store.repoId, releaseId: store.releaseId }),
      { mode: 0o600 },
    );
  return directory;
}
