import { z } from 'zod';
import { CLIPBOARD_TEXT_MAX_CHARS } from '../../../shared/ipc';
import { EXTENSION_URL_MAX } from '../../../shared/extension-url';

const id = z.string().min(1).max(128);
const label = z.string().min(1).max(512);
const handle = z.number().int().nonnegative().finite();
const small = z.string().max(4096);
type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
const data: z.ZodType<Json> = z.lazy(() => z.union([
  z.null(), z.boolean(), z.number().finite(), z.string(), z.array(data),
  z.custom<Record<string, unknown>>(value => value !== null && typeof value === 'object' && Object.getPrototypeOf(value) === Object.prototype).pipe(z.record(z.string(), data))
]));
const shape = <T extends z.ZodRawShape>(fields: T) => z.object(fields).strict();

export const registrationSchemas = {
  effects: shape({ id, label, group: small, params: z.array(data).max(32), frag: z.string().min(1).max(65_536), passes: z.number().int().min(1).max(8).optional(), keepOrig: z.boolean().optional(), backdrop: z.boolean().optional(), rawShader: z.boolean().optional(), viewportSafe: z.boolean().optional(), viewportPadding: z.array(z.string()).max(32).optional() }),
  transitions: shape({ id, label, group: small.optional(), params: z.array(data).max(32), frag: z.string().min(1).max(65_536), rawShader: z.boolean().optional() }),
  layers: shape({ id, label, version: z.number().int().nonnegative(), icon: small.optional(), color: small.optional(), width: z.number().finite().optional(), height: z.number().finite().optional(), params: z.array(data).max(32), defaults: z.record(z.string(), data).optional(), renderer: z.union([shape({ kind: z.literal('fragment'), fragment: z.string().max(65_536) }), shape({ kind: z.literal('mesh'), assetField: id }), shape({kind:z.literal('scene3d')}), shape({kind:z.literal('layer3d'),role:z.enum(['object','light','camera'])})]) }),
  // rootAttributes is accepted here but stripped for sandboxed themes in sandboxTheme() (sandbox-host.ts): the schema must not reject a field the host neutralizes.
  theme: shape({ id, name: label, scheme: z.enum(['light', 'dark', 'auto']), tokens: z.record(z.string().startsWith('--').max(128), small).optional(), darkTokens: z.record(z.string().startsWith('--').max(128), small).optional(), css: z.string().max(65_536).optional(), rootAttributes: z.record(z.string().max(128), small).optional() }),
  // inFields/looseModifiers/priority are deliberately NOT accepted: sandboxed
  // bindings are forced to priority >= 1000 and may not fire in fields, so an
  // untrusted extension cannot hijack a chord (see sandbox-host.test.ts).
  keybindings: shape({ key: id, command: id, args: z.array(data).max(32).optional(), repeat: z.boolean().optional() }),
  'media-defaults': shape({ anchor: shape({ x: z.number().finite(), y: z.number().finite() }) }),
  commands: shape({ id, label, category: small.optional(), kb: small.nullable().optional(), run: handle, when: handle.optional() }),
  status: shape({ id, text: handle, title: small.optional(), side: z.enum(['left', 'right']).optional(), onClick: handle.optional() }),
  palette: shape({ provider: handle }),
  menus: shape({ location: z.enum(['titlebar:right', 'panel:context', 'layer:context', 'timeline:context', 'viewer:context']), items: handle }),
  // Interest in an event name; listeners stay in the sandbox document.
  events: shape({ event: id }),
  // Only these layout hints cross to a sandboxed (iframe) panel; headless,
  // hideMoveHandle, moveSlot and the component/build/header handles deliberately
  // do not (panelInfo in shim-api.ts), so the schema rejects them.
  panels: shape({ id, title: label, icon: small.optional(), size: z.number().finite().optional(), min: z.number().finite().optional(), flush: z.boolean().optional(), noscroll: z.boolean().optional() })
} as const;

export type RegistrationKind = keyof typeof registrationSchemas;
export function parseRegistration(kind: string, value: unknown): Record<string, unknown> {
  const schema = registrationSchemas[kind as RegistrationKind];
  if (!schema) throw new Error(`Unknown sandbox registration: ${kind}`);
  return schema.parse(value) as Record<string, unknown>;
}

const anyArgs = z.array(data).max(32);
/* A toast's callbacks arrive as the calling document's handles (shim-api.ts). */
const toastOptions = z.object({ action: shape({ label, run: handle }).optional(), onDismiss: handle.optional() }).catchall(data);
const oneId = z.tuple([id]);
const storageKey = z.string().min(1).max(1024);
export const invokeSchemas: Record<string, z.ZodType> = {
  'commands.run': z.tuple([id]).rest(data),
  'project.apply': anyArgs, 'project.select': anyArgs, 'project.setTime': z.tuple([z.number().finite()]),
  'project.play': z.tuple([]), 'project.pause': z.tuple([]), 'project.undo': z.tuple([]), 'project.redo': z.tuple([]), 'project.snapshot': anyArgs,
  'transport.step': z.tuple([z.number().finite()]),
  'assets.pick': anyArgs, 'assets.import': z.tuple([z.custom<File>(value => typeof File !== 'undefined' && value instanceof File), data.optional()]), 'assets.get': oneId, 'assets.readText': oneId, 'assets.importUrl': z.tuple([z.string().max(EXTENSION_URL_MAX)]),
  'storage.get': z.tuple([storageKey]), 'storage.set': z.tuple([storageKey, data]), 'storage.delete': z.tuple([storageKey]),
  'ui.toast': z.tuple([data, toastOptions.optional()]), 'ui.confirm': anyArgs, 'ui.icon': anyArgs, 'ui.copy': z.tuple([z.string().max(CLIPBOARD_TEXT_MAX_CHARS)]), 'ui.openExternal': z.tuple([z.string().max(EXTENSION_URL_MAX)]),
  'panels.open': anyArgs, 'panels.close': oneId, 'panels.refresh': oneId, 'panels.isOpen': oneId,
  'keybindings.unbind': oneId, 'theme.activate': oneId,
  'palette.open': anyArgs, 'media.getImportDefaults': z.tuple([]),
  'events.emit': z.tuple([id, data]), 'extensions.setUp': oneId
};
/** The File argument 0 of an `assets.import` call carries: the RPC byte limit skips it, and the handler caps it on `file.size` before a byte is read. */
export function importedFile(method: unknown, args: unknown[]): File | undefined {
  if (method !== 'invoke' || args[0] !== 'assets' || args[1] !== 'import' || !Array.isArray(args[2])) return undefined;
  const file: unknown = args[2][0];
  return typeof File !== 'undefined' && file instanceof File ? file : undefined;
}
export function parseInvoke(namespace: string, method: string, args: unknown): unknown[] {
  const schema = invokeSchemas[`${namespace}.${method}`];
  if (!schema) throw new Error(`Sandbox method unavailable: ${namespace}.${method}`);
  return schema.parse(args) as unknown[];
}

export const paletteEntriesSchema = z.array(shape({ id, label, category: label, kb: small.nullable().optional(), run: handle })).max(200);
export const menuEntriesSchema = z.array(z.union([
  z.literal('-'), shape({ header: label }),
  shape({ label, icon: small.optional(), kb: small.nullable().optional(), on: z.boolean().optional(), disabled: z.boolean().optional(), run: handle.optional() })
])).max(200);

const hostEventSchemas: Record<string, z.ZodType> = {
  'project:changed': shape({ kind: z.enum(['values', 'structure', 'project', 'assets', 'library', 'history', 'replace']) }),
  selection: z.unknown(), time: z.number().finite(), transport: shape({ playing: z.boolean() }),
  theme: shape({ id, scheme: z.enum(['light', 'dark']) }),
  'extensions:changed': shape({ ids: z.array(id).max(2000), reason: small })
};
export function parseHostEvent(event: string, payload: unknown): unknown {
  const schema = hostEventSchemas[event];
  if (!schema) throw new Error(`Host event unavailable: ${event}`);
  return schema.parse(payload);
}
