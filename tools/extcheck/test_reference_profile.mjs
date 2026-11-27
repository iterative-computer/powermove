import assert from "node:assert/strict";
import {
  mkdtemp,
  mkdir,
  writeFile,
  readFile,
  readdir,
  rm,
} from "node:fs/promises";
import path from "node:path";
import { prepareReferenceExtension } from "./reference-profile.mjs";

const root = await mkdtemp("/tmp/pm-extcheck/reference-profile-test-");
try {
  const input = path.join(root, "input");
  await mkdir(input);
  await writeFile(path.join(input, "manifest.json"), '{"id":"synthetic"}');
  const store = {
    repoId: "synthetic-repository",
    releaseId: "synthetic-release",
  };
  const directory = await prepareReferenceExtension(
    path.join(root, "store"),
    input,
    "synthetic",
    store,
  );
  assert.deepEqual(
    JSON.parse(
      await readFile(path.join(directory, ".powermove-store.json"), "utf8"),
    ),
    store,
  );
  assert.deepEqual(await readdir(input), ["manifest.json"]);
  const local = await prepareReferenceExtension(
    path.join(root, "local"),
    input,
    "synthetic",
  );
  assert.deepEqual(await readdir(local), ["manifest.json"]);
  await assert.rejects(
    prepareReferenceExtension(
      path.join(root, "bad"),
      input,
      "../escape",
      store,
    ),
  );
  await assert.rejects(
    prepareReferenceExtension(path.join(root, "bad"), input, "synthetic", {}),
  );
  console.log(
    "PASS: disposable Store marker, local isolation and unchanged source",
  );
} finally {
  await rm(root, { recursive: true, force: true });
}
