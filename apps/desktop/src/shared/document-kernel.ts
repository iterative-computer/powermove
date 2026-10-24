/** Owner-scoped transport for the document evaluator in the interactive utility. */
export const DOCUMENT_KERNEL_IPC = {
  call: 'engine:document-kernel', begin: 'engine:document-begin',
  write: 'engine:document-write', commit: 'engine:document-commit', abort: 'engine:document-abort', digestBegin: 'engine:document-digest-begin', digestWrite: 'engine:document-digest-write', digestEnd: 'engine:document-digest-end', digestBatch: 'engine:document-digest-batch'
} as const;

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
