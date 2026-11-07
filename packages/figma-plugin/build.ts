const result = await Bun.build({
  entrypoints: ['code.ts'],
  outdir: 'dist',
  target: 'browser',
  format: 'iife',
  minify: true,
});
if (!result.success) throw new AggregateError(result.logs, 'Figma plugin build failed');
export {};
