/** Owner-scoped transport for the document evaluator in the interactive utility. */
export const DOCUMENT_KERNEL_IPC = {
  call: 'engine:document-kernel', begin: 'engine:document-begin',
  write: 'engine:document-write', commit: 'engine:document-commit', abort: 'engine:document-abort', digestBegin: 'engine:document-digest-begin', digestWrite: 'engine:document-digest-write', digestEnd: 'engine:document-digest-end', digestBatch: 'engine:document-digest-batch'
} as const;

/** A document stream's largest chunk, and how many main holds unwritten:
 *  main writes each chunk to its staging file and hashes it before the next,
 *  so fewer, larger round trips keep it busy (1 MiB two at a time left a
 *  180 MB stream waiting on replies for half a second). */
export const DOCUMENT_CHUNK_BYTES = 4 * 1024 * 1024;
export const DOCUMENT_QUEUED_WRITES = 4;

export interface DocumentKernelReceipt {
  pending: false;
  revision: number;
  hash: string;
  integrity: string;
}

export interface DocumentKernelBridge {
  /** Host restores document, context and stable handles without a renderer task. */
  readonly automaticRecovery?: true;
  /** The trusted host supplies the owner. The page cannot name a file or another window. */
  call(request: string, bytes?: Uint8Array): string;
  begin(): Promise<string>;
  write(token: string, chunk: Uint8Array): Promise<void>;
  commit(token: string, document: string, revision: number): Promise<DocumentKernelReceipt>;
  abort(token: string): Promise<void>;
  beginDigest(prefix: string): Promise<string>;
  writeDigest(token: string, chunk: Uint8Array): Promise<void>;
  endDigest(token: string): Promise<string>;
  digestBatch(instructions: import('./document-digest').DigestInstruction[]): Promise<string[]>;
}
