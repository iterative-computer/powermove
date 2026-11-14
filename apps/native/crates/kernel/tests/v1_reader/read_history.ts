// Runs the old app's own history reader (v1.2.0 / v1.3.2 sources, whose
// history-memory.ts and history.ts import paths are identical) on a history
// member written by the native writer, and compares what it reads with what
// the same reader reads from the unpacked history (the writer before the
// packed form): the old app keeps no unknown members either way.
//
//   PM_V1_REF=<checkout of v1.2.0 or v1.3.2> bun read_history.ts PACKED.json PLAIN.json
//
// PACKED.json: the `history` member of a saved project (the dictionary form).
// PLAIN.json:  the same history unpacked (`HistorySnapshot::to_saved`).
// Prints one line per reader path and exits non-zero on any difference.
const ref = process.env.PM_V1_REF;
if (!ref) throw new Error('PM_V1_REF is not set');
const core = `${ref}/apps/desktop/src/renderer/src/legacy/core`;
const { unpackHistory } = await import(`${ref}/apps/desktop/src/shared/history-memory.ts`);
const { install } = await import(`${core}/history.ts`);

const [packedPath, plainPath, compId] = process.argv.slice(2);
const read = async (path: string) => JSON.parse(await Bun.file(path).text());
const packed = await read(packedPath!);
const plain = await read(plainPath!);

// Key order is not part of the JSON value: compare canonically.
const canonical = (value: any): any =>
  Array.isArray(value)
    ? value.map(canonical)
    : value && typeof value === 'object'
      ? Object.fromEntries(Object.keys(value).sort().map((k) => [k, canonical(value[k])]))
      : value;
const same = (a: any, b: any) => JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));

// The registry pieces history.ts touches while importing and exporting. A
// history saved with composition paths never reaches the composition
// canonicalizer.
const registry = () => {
  let n = 0;
  const PM: any = {
    proj: { id: 'P', compId: compId ?? 'C', revision: 0 },
    bus: { emit() {} },
    uid: (prefix: string) => `${prefix}-${n++}`,
    store: { get() {}, set() {} },
    sel: { keys: [] },
    invalidate() {},
    touch() {},
    takes: { all: () => [] },
  };
  install(PM);
  return PM;
};

let failed = false;
const report = (what: string, ok: boolean, yes = 'same history', no = 'DIFFERENT') => {
  console.log(`${what}: ${ok ? yes : no}`);
  if (!ok) failed = true;
};

report('unpackHistory', same(unpackHistory(structuredClone(packed)), plain));

// What the old app reads from the unpacked history.
const baseline = registry();
report('PM.hist.import of the unpacked history', baseline.hist.import(structuredClone(plain)) === true, 'accepted', 'REFUSED');
const expected = baseline.hist.export({ copy: false });

const direct = registry();
const imported = direct.hist.import(structuredClone(packed));
report('PM.hist.import', imported === true, 'accepted', 'REFUSED');
report('PM.hist.import then export', same(direct.hist.export({ copy: false }), expected));

// The open-a-file path: prepareImport, then import with its token.
const opened = registry();
const token = await opened.hist.prepareImport(structuredClone(packed), () => true, compId ?? 'C');
report('PM.hist.prepareImport', token !== undefined, 'accepted', 'REFUSED');
opened.hist.import(undefined, token);
report('prepareImport then import then export', same(opened.hist.export({ copy: false }), expected));

process.exit(failed ? 1 : 0);
