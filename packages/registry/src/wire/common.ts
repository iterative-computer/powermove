import { z } from 'zod';
import { EXTENSION_ID, EXTENSION_PERMISSIONS, EXTENSION_VERSION, EXTENSION_VAR_KEY } from '../manifest';

export const Category = z.enum(['effects', 'transitions', 'panels', 'themes', 'commands', 'layers', 'tools']);
export const Permission = z.enum(EXTENSION_PERMISSIONS);
export type Permission = z.infer<typeof Permission>;
export const Visibility = z.enum(['public', 'unlisted']);
export const Moderation = z.enum(['none', 'hidden', 'removed']);
export const Handle = z.string().regex(/^[a-z0-9][a-z0-9-]{1,38}$/);
export const Slug = z.string().regex(EXTENSION_ID);
export const Version = z.string().regex(EXTENSION_VERSION);
export const Sha1 = z.string().regex(/^[a-f0-9]{40}$/);
export const Sha256 = z.string().regex(/^[a-f0-9]{64}$/);
export const Uuid = z.uuid();
export const IsoDate = z.iso.datetime();
export const VarDecl = z.object({
  key: z.string().regex(EXTENSION_VAR_KEY),
  label: z.string().min(1).max(80),
  secret: z.boolean().optional(),
  required: z.boolean().optional(),
  hint: z.string().max(200).optional()
});
export type VarDecl = z.infer<typeof VarDecl>;
