import { copyFile } from 'node:fs/promises';
// Keep the standalone site's theme identical to the editor's source of truth.
await copyFile(new URL('../../css/tokens.css', import.meta.url), new URL('../public/editor-tokens.css', import.meta.url));
