// Opens a project file the native writer saved with the old app's own code,
// as the old app opens a file on a machine whose media store lacks the
// project's media: decode the container, restore its embedded media into a
// fresh store (before the history, as the app does), import the history,
// Undo the last step, Save As through the old packer, and reopen that file.
// It checks that media only an undo step refers to survives every stage.
//
//   V1_SRC=<exported apps/desktop/src of v1.2.0 or v1.3.2> \
//     bun read_file.ts FILE.pmv MEDIA_ID
//
// Exits non-zero, and says where, if the media is missing at any stage.
const src = process.env.V1_SRC;
if (!src) throw new Error('V1_SRC is not set');
const core = `${src}/renderer/src/legacy/core`;
const projectFile = await import(`${core}/project-file.ts`);
const { install } = await import(`${core}/history.ts`);

const [file, mediaId] = process.argv.slice(2);
const original = await Bun.file(file!).arrayBuffer();

// A fresh media store, as on a machine that never had this project.
const blobs = new Map<string, Blob>();
const store = {
  async get(asset: any) {
    return blobs.get(asset?.id) ?? null;
  },
  async put(id: string, blob: Blob) {
    blobs.set(id, blob);
    return true;
  },
};

let failed = false;
const report = (what: string, ok: boolean) => {
  console.log(`${what}: ${ok ? 'yes' : 'NO'}`);
  if (!ok) failed = true;
};
const bytesOf = async (blob: Blob | undefined) =>
  blob ? new Uint8Array(await blob.arrayBuffer()) : undefined;
const same = (a?: Uint8Array, b?: Uint8Array) =>
  !!a && !!b && a.length === b.length && a.every((v, i) => v === b[i]);

// 1. Open: decode, then restore media before the history is imported.
const document = await projectFile.unpackProjectFileBlob(new Blob([original]));
const embedded = (document.containerMedia ?? []).find((m: any) => m.id === mediaId);
report(`the file embeds ${mediaId}`, !!embedded);
const savedBytes = await bytesOf(embedded?.data);
await projectFile.restoreProjectFileMedia(document, store);
report('open restores it into the media store', same(await bytesOf(blobs.get(mediaId!)), savedBytes));

// 2. The history, through the app's file-open path.
let n = 0;
const PM: any = {
  proj: structuredClone(document.proj),
  bus: { emit() {} },
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
install(PM);
const token = await PM.hist.prepareImport(document.history, () => true, PM.proj.compId);
report('the history imports', token !== undefined);
PM.hist.import(undefined, token);

// 3. Undo the last step (Remove Media): the asset is back in the project.
report('undo runs', PM.hist.undo() === true);
report(`undo restores asset ${mediaId}`, !!PM.proj.assets?.[mediaId!]);

// 4. Save As through the old packer, and reopen what it wrote.
const snapshot = { v: document.v, proj: PM.proj, history: PM.hist.export({ copy: false }), ws: document.ws ?? null };
const written = await projectFile.packProjectFileBlob(snapshot, store);
const reopened = await projectFile.unpackProjectFileBlob(written);
const kept = (reopened.containerMedia ?? []).find((m: any) => m.id === mediaId);
report('Save As keeps its bytes', same(await bytesOf(kept?.data), savedBytes));

process.exit(failed ? 1 : 0);
