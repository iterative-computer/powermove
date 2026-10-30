// occt-import-js ships no types: the parts Powermove uses.
declare module 'occt-import-js' {
  type Occt = { ReadFile(format: 'step' | 'iges' | 'brep', content: Uint8Array, params: Record<string, unknown> | null): any };
  const occtimportjs: (overrides?: { locateFile?: (path: string) => string }) => Promise<Occt>;
  export default occtimportjs;
}
