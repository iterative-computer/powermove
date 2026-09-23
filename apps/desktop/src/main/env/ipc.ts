/*
 * `vars:*` handlers. Renderer-reachable IPC is hostile (store plan §2.6):
 * every payload is zod-validated, the sender must be a trusted app frame, and
 * no handler returns a stored value except `vars:values`, which exists to
 * deliver values to `api.vars` and is readable by any extension in the shared
 * renderer realm (§2.2b). Reveal is a native dialog; Copy runs in main.
 */
import { BrowserWindow, clipboard, dialog, type IpcMain, type IpcMainInvokeEvent, type MessageBoxOptions } from 'electron';
import { z } from 'zod';

import { IpcValidationError } from '../../shared/guards';
import { EXTENSION_ID, EXTENSION_VAR_KEY, type ExtensionRecord, type ExtensionVarDecl } from '../../shared/extensions';
import { VARS_IPC, VARS_VALUE_MAX_CHARS, type VarsStatus } from '../../shared/vars-ipc';
import type { ExtensionRegistry } from '../extensions/registry';
import type { VarsService } from './service';

type Sender = Pick<IpcMainInvokeEvent, 'sender' | 'senderFrame'>;
type ShowMessageBox = (window: BrowserWindow | null, options: MessageBoxOptions) => Promise<{ response: number }>;

export interface VarsIpcOptions {
  registry: Pick<ExtensionRegistry, 'list' | 'refresh' | 'emitChanged'>;
  vars: VarsService;
  isTrusted(event: Sender): boolean;
  /** Test seams; production callers leave these unset. */
  showMessageBox?: ShowMessageBox;
  writeClipboard?: (text: string) => void;
  windowFor?: (event: Sender) => BrowserWindow | null;
}

const id = z.string().regex(EXTENSION_ID);
const key = z.string().regex(EXTENSION_VAR_KEY);
export const varsSchemas = {
  id: z.strictObject({ id }),
  key: z.strictObject({ id, key }),
  set: z.strictObject({
    id,
    key,
    value: z.string().max(VARS_VALUE_MAX_CHARS).refine((value) => !value.includes('\0'), 'value contains NUL')
  })
} as const;

function parse<T>(schema: z.ZodType<T>, channel: string, payload: unknown): T {
  const result = schema.safeParse(payload);
  if (!result.success) throw new IpcValidationError(channel, result.error.issues.map((issue) => issue.message).join('; '));
  return result.data;
}

/** Everything but the last four characters, and all of it when the value is short. */
export function maskValue(value: string): string {
  const dots = '••••••••';
  return value.length > 8 ? `${dots}${value.slice(-4)}` : dots;
}

const defaultShow: ShowMessageBox = (window, options) =>
  window ? dialog.showMessageBox(window, options) : dialog.showMessageBox(options);

const defaultWindowFor = (event: Sender): BrowserWindow | null => {
  const window = BrowserWindow.fromWebContents(event.sender);
  return window && !window.isDestroyed() ? window : null;
};

function userRecord(registry: VarsIpcOptions['registry'], extensionId: string): ExtensionRecord | null {
  const record = registry.list().find((item) => item.id === extensionId);
  return record && record.scope === 'user' ? record : null;
}

export function registerVarsIpc(ipcMain: Pick<IpcMain, 'handle'>, options: VarsIpcOptions): void {
  const { registry, vars } = options;
  const show = options.showMessageBox ?? defaultShow;
  const windowFor = options.windowFor ?? defaultWindowFor;
  const writeClipboard = options.writeClipboard ?? ((text: string) => clipboard.writeText(text));

  const requireTrusted = (event: Sender, channel: string): void => {
    if (!options.isTrusted(event)) throw new IpcValidationError(channel, 'untrusted sender');
  };

  const declared = (extensionId: string): { record: ExtensionRecord; decls: ExtensionVarDecl[] } => {
    const record = userRecord(registry, extensionId);
    const decls = record?.manifest?.vars ?? [];
    if (!record || decls.length === 0) throw new Error('This extension has no values to set.');
    return { record, decls };
  };

  const declOf = (decls: readonly ExtensionVarDecl[], name: string): ExtensionVarDecl => {
    const decl = decls.find((item) => item.key === name);
    if (!decl) throw new Error(`${name} is not declared by this extension.`);
    return decl;
  };

  /* A change can turn the extension on or off, and a running one needs its
     new values: re-resolve, then reload it in every window. */
  const changed = async (extensionId: string, decls: readonly ExtensionVarDecl[]): Promise<VarsStatus> => {
    await registry.refresh([extensionId]);
    registry.emitChanged({ ids: [extensionId], reason: 'reload' });
    return vars.status(extensionId, decls);
  };

  ipcMain.handle(VARS_IPC.status, async (event, payload: unknown): Promise<VarsStatus> => {
    requireTrusted(event, VARS_IPC.status);
    const request = parse(varsSchemas.id, VARS_IPC.status, payload);
    const { decls } = declared(request.id);
    return vars.status(request.id, decls);
  });

  ipcMain.handle(VARS_IPC.set, async (event, payload: unknown): Promise<VarsStatus> => {
    requireTrusted(event, VARS_IPC.set);
    const request = parse(varsSchemas.set, VARS_IPC.set, payload);
    const { decls } = declared(request.id);
    await vars.set(request.id, declOf(decls, request.key), request.value);
    return changed(request.id, decls);
  });

  ipcMain.handle(VARS_IPC.delete, async (event, payload: unknown): Promise<VarsStatus> => {
    requireTrusted(event, VARS_IPC.delete);
    const request = parse(varsSchemas.key, VARS_IPC.delete, payload);
    const { decls } = declared(request.id);
    await vars.delete(request.id, declOf(decls, request.key).key);
    return changed(request.id, decls);
  });

  ipcMain.handle(VARS_IPC.reveal, async (event, payload: unknown): Promise<void> => {
    requireTrusted(event, VARS_IPC.reveal);
    const request = parse(varsSchemas.key, VARS_IPC.reveal, payload);
    const { decls } = declared(request.id);
    const decl = declOf(decls, request.key);
    const value = await vars.read(request.id, decls, decl.key);
    if (value === null) throw new Error(`${decl.label} has no value to show.`);
    const { response } = await show(windowFor(event), {
      type: 'none',
      message: decl.label,
      detail: maskValue(value),
      buttons: ['Copy', 'Done'],
      defaultId: 1,
      cancelId: 1,
      noLink: true
    });
    if (response === 0) writeClipboard(value);
  });

  ipcMain.handle(VARS_IPC.values, async (event, payload: unknown): Promise<Record<string, string>> => {
    requireTrusted(event, VARS_IPC.values);
    const request = parse(varsSchemas.id, VARS_IPC.values, payload);
    const record = userRecord(registry, request.id);
    const decls = record?.manifest?.vars ?? [];
    if (!record || decls.length === 0) return {};
    return (await vars.resolve(request.id, decls)).values;
  });
}

/**
 * The `ext:remove` hook: when the extension has stored values, ask whether to
 * keep them. Returns the deletion to run once the folder is gone, so a failed
 * removal never loses values.
 */
export function createRemoveValuesPrompt(options: Pick<VarsIpcOptions, 'registry' | 'vars' | 'showMessageBox' | 'windowFor'>) {
  const show = options.showMessageBox ?? defaultShow;
  const windowFor = options.windowFor ?? defaultWindowFor;
  return async (event: Sender, extensionId: string): Promise<(() => Promise<void>) | undefined> => {
    const record = userRecord(options.registry, extensionId);
    if (!record || !(await options.vars.hasValues(extensionId))) return undefined;
    const name = record.manifest?.name ?? extensionId;
    const { response } = await show(windowFor(event), {
      type: 'question',
      message: `Also delete the values you entered for ${name}?`,
      detail: 'If you keep them, they’re used again when you add this extension back.',
      buttons: ['Keep', 'Delete'],
      defaultId: 0,
      cancelId: 0,
      noLink: true
    });
    return response === 1 ? () => options.vars.forget(extensionId) : undefined;
  };
}
