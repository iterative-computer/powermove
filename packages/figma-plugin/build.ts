const result = await Bun.build({
  entrypoints: ['code.ts'],
  outdir: 'dist',
  target: 'browser',
  format: 'iife',
  minify: true,
});
if (!result.success) throw new AggregateError(result.logs, 'Figma plugin build failed');
// Figma's UI iframe cannot import our workspace CSS. Bundle the same tokens
// used by desktop panels so both themes follow the app's control materials.
const template = await Bun.file('ui.html').text();
const tokens = await Bun.file('../tokens/tokens.css').text();
await Bun.write('dist/ui.html', template.replace('/* POWERMOVE_TOKENS */', () => tokens));
export {};
