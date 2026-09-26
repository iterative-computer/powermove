import { z } from 'zod';

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
  effects: shape({ id, label, group: small, params: z.array(data).max(32), frag: z.string().min(1).max(65_536), passes: z.number().int().min(1).max(8).optional(), keepOrig: z.boolean().optional(), rawShader: z.boolean().optional() }),
  transitions: shape({ id, label, group: small.optional(), params: z.array(data).max(32), frag: z.string().min(1).max(65_536), rawShader: z.boolean().optional() }),
  layers: shape({ id, label, version: z.number().int().nonnegative(), icon: small.optional(), color: small.optional(), width: z.number().finite().optional(), height: z.number().finite().optional(), params: z.array(data).max(32), defaults: z.record(z.string(), data).optional(), renderer: z.union([shape({ kind: z.literal('fragment'), fragment: z.string().max(65_536) }), shape({ kind: z.literal('mesh'), assetField: id })]) }),
  theme: shape({ id, name: label, scheme: z.enum(['light', 'dark', 'auto']), tokens: z.record(z.string().startsWith('--').max(128), small).optional(), darkTokens: z.record(z.string().startsWith('--').max(128), small).optional(), css: z.string().max(65_536).optional() }),
  keybindings: shape({ key: id, command: id, args: z.array(data).max(32).optional(), repeat: z.boolean().optional() }),
  'media-defaults': shape({ anchor: shape({ x: z.number().finite(), y: z.number().finite() }) }),
  commands: shape({ id, label, category: small.optional(), kb: small.nullable().optional(), run: handle, when: handle.optional() }),
  status: shape({ id, text: handle, title: small.optional(), side: z.enum(['left', 'right']).optional(), onClick: handle.optional() }),
  palette: shape({ provider: handle }),
  menus: shape({ location: z.enum(['titlebar:right', 'panel:context', 'layer:context', 'timeline:context', 'viewer:context']), items: handle }),
  events: shape({ event: id, fn: handle }),
  panels: shape({ id, title: label, icon: small.optional(), size: z.number().finite().optional(), min: z.number().finite().optional(), flush: z.boolean().optional(), noscroll: z.boolean().optional() })
} as const;

export type RegistrationKind = keyof typeof registrationSchemas;
export function parseRegistration(kind: string, value: unknown): Record<string, unknown> {
  const schema = registrationSchemas[kind as RegistrationKind];
  if (!schema) throw new Error(`Unknown sandbox registration: ${kind}`);
  return schema.parse(value) as Record<string, unknown>;
}

const anyArgs = z.array(data).max(32);
const oneId = z.tuple([id]);
const storageKey = z.string().min(1).max(1024);
export const invokeSchemas: Record<string, z.ZodType> = {
  'commands.run': z.tuple([id]).rest(data),
  'project.apply': anyArgs, 'project.select': anyArgs, 'project.setTime': z.tuple([z.number().finite()]),
  'project.play': z.tuple([]), 'project.pause': z.tuple([]), 'project.undo': z.tuple([]), 'project.redo': z.tuple([]), 'project.snapshot': anyArgs,
  'transport.step': z.tuple([z.number().finite()]),
  'assets.pick': anyArgs, 'assets.import': z.tuple([z.custom<File>(value => typeof File !== 'undefined' && value instanceof File), data.optional()]), 'assets.get': oneId, 'assets.readText': oneId,
  'storage.get': z.tuple([storageKey]), 'storage.set': z.tuple([storageKey, data]), 'storage.delete': z.tuple([storageKey]),
  'ui.toast': anyArgs, 'ui.confirm': anyArgs, 'ui.icon': anyArgs,
  'panels.open': anyArgs, 'panels.close': oneId, 'panels.refresh': oneId,
  'keybindings.unbind': oneId, 'theme.activate': oneId,
  'palette.open': anyArgs, 'media.getImportDefaults': z.tuple([]),
  'events.emit': z.tuple([id, data]), 'extensions.setUp': oneId
};
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
