import { open } from 'node:fs/promises';

/** Reads `length` bytes at `offset` of a project transaction file and checks
 * they are a Powermove document. Parsing a document with its undo history can
 * take hundreds of milliseconds, so the desktop runs this on the persistence
 * worker thread (persistence-worker.ts); other hosts call it directly. */
export async function validateProjectDocument(file: string, offset: number, length: number): Promise<void> {
  if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(length) || length < 2) throw new Error('Invalid project document range');
  const handle = await open(file, 'r');
  let bytes: Buffer;
  try {
    bytes = Buffer.allocUnsafe(length);
    for (let read = 0; read < length;) {
      const { bytesRead } = await handle.read(bytes, read, length - read, offset + read);
      if (!bytesRead) throw new Error('The project container is truncated.');
      read += bytesRead;
    }
  } finally { await handle.close(); }
  const document = JSON.parse(bytes.toString('utf8'));
  const project = document?.proj || document;
  if (!project || !Array.isArray(project.layers) || !Number.isFinite(project.w) || !Number.isFinite(project.h)) throw new Error('This is not a Powermove project.');
}

export type ProjectDocumentValidator = typeof validateProjectDocument;
