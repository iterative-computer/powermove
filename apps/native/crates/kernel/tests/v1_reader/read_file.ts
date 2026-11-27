// Opens a project file the native writer saved with the old app's own code,
// as the old app opens a file on a machine whose media store lacks the
// project's media: decode the container, restore its embedded media into a
// fresh store (before the history, as the app does), bring the project into
// the composition shape and import the history with the app's own
// compositions module installed (it maps each step onto the open
// composition and prepares Undo), Undo the last step, Save As through the
// old packer, then open that file the same way on another fresh store.
// It checks that a picture only an undo step refers to survives every
// stage: its bytes, its asset, and its layer in the open composition.
//
//   V1_SRC=<exported apps/desktop/src of v1.2.0 or v1.3.2> \
//     bun read_file.ts FILE.pmv MEDIA_ID LAYER_ID
//
// Prints `release: <tag>` for the release whose modules it loaded (each
// module's git blob id is checked against the tag's). Exits 1, saying
// where, if the picture is missing at any stage; 2 if the sources are not
// exactly one of the two releases.
const src = process.env.V1_SRC;
if (!src) {
  console.log('V1_SRC is not set');
  process.exit(2);
}
const core = `${src}/renderer/src/legacy/core`;

// The git blob ids of the modules this check runs, at each release tag.
const RELEASES: Record<string, Record<string, string>> = {
  'v1.2.0': {
    'project-file.ts': '79ee5019fd6f666ceffa456832e7603ffc3863fd',
    'history.ts': '3bdec10de1f91c673fdb841eebddbc759551eed7',
    'compositions.ts': '50c2172671d3d7d3ae85cdc8f563498ff7888e68',
  },
  'v1.3.2': {
    'project-file.ts': '0079aef9ad7f8c62f7978259571967ef24a01fb3',
    'history.ts': '3bdec10de1f91c673fdb841eebddbc759551eed7',
    'compositions.ts': '8a46eba2672277522ea5cdc90ce5cc7403c6a8a6',
  },
};
const blobId = async (path: string) => {
  const bytes = new Uint8Array(await Bun.file(path).arrayBuffer());
  const hasher = new Bun.CryptoHasher('sha1');
  hasher.update(`blob ${bytes.length}\0`);
  hasher.update(bytes);
  return hasher.digest('hex');
};
const ids: Record<string, string> = {};
for (const name of Object.keys(RELEASES['v1.2.0']!)) ids[name] = await blobId(`${core}/${name}`);
const release = Object.entries(RELEASES).find(([, files]) =>
  Object.entries(files).every(([name, id]) => ids[name] === id),
)?.[0];
if (!release) {
  console.log(`not v1.2.0 or v1.3.2 sources: ${JSON.stringify(ids)}`);
  process.exit(2);
}
console.log(`release: ${release}`);

const projectFile = await import(`${core}/project-file.ts`);
const history = await import(`${core}/history.ts`);
const compositions = await import(`${core}/compositions.ts`);

const [file, mediaId, layerId] = process.argv.slice(2);

let failed = false;
const report = (what: string, ok: boolean) => {
  console.log(`${what}: ${ok ? 'yes' : 'NO'}`);
  if (!ok) failed = true;
};
const bytesOf = async (blob: Blob | undefined | null) =>
  blob ? new Uint8Array(await blob.arrayBuffer()) : undefined;
const same = (a?: Uint8Array, b?: Uint8Array) =>
  !!a && !!b && a.length === b.length && a.every((v, i) => v === b[i]);

/** A media store with nothing in it, as on a machine that never had this project. */
const emptyStore = () => {
  const blobs = new Map<string, Blob>();
  return {
    blobs,
    async get(asset: any) {
      return blobs.get(asset?.id) ?? null;
    },
    async put(id: string, blob: Blob) {
      blobs.set(id, blob);
      return true;
    },
  };
};

/** The picture's layer in the open composition, showing its asset. */
const pictureShown = (proj: any) =>
  !!proj?.assets?.[mediaId!] &&
  (proj?.layers ?? []).some((layer: any) => layer.id === layerId && layer.d?.asset === mediaId);

/**
 * Opens a container the way the app does: decode, restore media into an
 * empty store, normalize the compositions, import the history with the
 * compositions and history modules installed. Returns the registry.
 */
const open = async (blob: Blob, what: string, bytes?: Uint8Array) => {
  const document = await projectFile.unpackProjectFileBlob(blob);
  const embedded = (document.containerMedia ?? []).find((m: any) => m.id === mediaId);
  report(`${what}: the file embeds ${mediaId}`, !!embedded);
  const saved = bytes ?? (await bytesOf(embedded?.data));
  const store = emptyStore();
  await projectFile.restoreProjectFileMedia(document, store);
  report(`${what}: open restores it into the media store`, same(await bytesOf(store.blobs.get(mediaId!)), saved));
  let n = 0;
  const listeners = new Map<string, Array<(...args: any[]) => void>>();
  const PM: any = {
    proj: null,
    time: 0,
    clamp: (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v)),
    bus: {
      on(event: string, fn: (...args: any[]) => void) {
        listeners.set(event, [...(listeners.get(event) ?? []), fn]);
      },
      emit(event: string, ...args: any[]) {
        for (const fn of listeners.get(event) ?? []) fn(...args);
      },
    },
    uid: (prefix: string) => `${prefix}-${n++}`,
    store: { get() {}, set() {} },
    sel: { keys: [] },
    invalidate() {},
    touch() {},
    takes: { all: () => [] },
    replaceProject(next: any) {
      PM.proj = next;
    },
  };
  compositions.install(PM);
  history.install(PM);
  const proj = structuredClone(document.proj);
  PM.Comps.normalize(proj);
  PM.proj = proj;
  const token = await PM.hist.prepareImport(document.history, () => true, PM.proj.compId);
  report(`${what}: the history imports`, token !== undefined);
  PM.hist.import(undefined, token);
  return { PM, document, store, saved };
};

try {
  // 1. Open the native writer's file.
  const first = await open(new Blob([await Bun.file(file!).arrayBuffer()]), 'open');

  // 2. Undo the last step (Remove Media) through the app's own path: the
  //    picture's asset and its layer are back in the open composition.
  report('undo runs', first.PM.hist.undo() === true);
  report(`undo shows ${layerId} with ${mediaId} again`, pictureShown(first.PM.proj));

  // 3. Save As through the old packer.
  const snapshot = {
    v: first.document.v,
    proj: first.PM.proj,
    history: first.PM.hist.export({ copy: false }),
    ws: first.document.ws ?? null,
  };
  const written = await projectFile.packProjectFileBlob(snapshot, first.store);

  // 4. Open what it wrote on another machine: the picture is shown with its bytes.
  const second = await open(written, 'reopen', first.saved);
  report(`reopen shows ${layerId} with ${mediaId}`, pictureShown(second.PM.proj));
} catch (error) {
  report(`the old app ran to the end (${error})`, false);
}

process.exit(failed ? 1 : 0);
