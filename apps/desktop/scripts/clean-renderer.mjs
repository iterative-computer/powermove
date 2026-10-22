import { rm } from 'node:fs/promises';
import path from 'node:path';

// Package only the current bundle; local builds retain older lazy chunks for
// editor windows that are still running.
await rm(path.resolve(import.meta.dirname, '../out/renderer'), { recursive: true, force: true });
