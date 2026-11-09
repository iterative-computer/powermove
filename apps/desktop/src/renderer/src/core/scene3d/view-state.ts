import { z } from 'zod';

const vector=z.tuple([z.number().finite().min(-1e8).max(1e8),z.number().finite().min(-1e8).max(1e8),z.number().finite().min(-1e8).max(1e8)]);
const schema=z.object({
  layout:z.union([z.literal(1),z.literal(2),z.literal(4)]),
  active:z.number().int().min(0).max(3),
  panes:z.array(z.object({view:z.enum(['camera','front','back','left','right','top','bottom','custom']),
    pose:z.object({position:vector,target:vector,zoom:z.number().finite().positive().max(1e6)}).strict().nullable()
  }).strict()).min(1).max(4)
}).strict().refine(v=>v.panes.length===v.layout && v.active<v.layout,'3D views must match their layout');

export type StoredViews = z.output<typeof schema>;
export const parseStoredViews=(raw:unknown):StoredViews=>schema.parse(raw);
