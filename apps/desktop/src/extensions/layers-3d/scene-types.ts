import type { SceneLight, SceneObject, SceneSource } from 'powermove';

export type { SceneChannel } from 'powermove';
export type Primitive = Extract<SceneSource, { primitive: string }>['primitive'];
export type LightType = SceneLight['type'];
export type MapSlot = keyof SceneObject['material']['maps'];
