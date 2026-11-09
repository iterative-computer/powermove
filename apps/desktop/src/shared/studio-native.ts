/** Native Studio wire protocol. JSON structure and raw binary never share a base64 envelope. */
export const STUDIO_NATIVE_IPC = {
  request: 'studio-native:request',
  release: 'studio-native:release',
  status: 'studio-native:status',
} as const;
export const STUDIO_PROTOCOL = 1;
export const STUDIO_MAX_JSON = 4 * 1024 * 1024;
export const STUDIO_MAX_BINARY = 256 * 1024 * 1024;
export type StudioNativeCommand = { op: string; [key: string]: unknown };
const STUDIO_COMMANDS = new Set(['hello', 'scene', 'preset', 'patch', 'screen', 'render', 'pick', 'bounds', 'prepared', 'preparedBegin', 'preparedChunk', 'preparedCommit', 'preparedPatch', 'preparedView', 'cancelRefine', 'textures', 'snapshot', 'importFbx']);
/** Page-facing commands exclude lifecycle/texture acknowledgements owned by main. */
export function isStudioNativeCommand(command: unknown): command is StudioNativeCommand {
  return !!command && typeof command === 'object' && 'op' in command && typeof command.op === 'string' && STUDIO_COMMANDS.has(command.op);
}
export type StudioNativeReply = {
  result: any;
  binary: Uint8Array;
  /** Changes whenever the process restarts; clients must upload their scene again. */
  generation: number;
  roundTripMs: number;
};
export type StudioNativeStatus = {
  available: boolean;
  running: boolean;
  generation: number;
  failure: string | null;
};
export type StudioNativeTextureInfo = { session: string; frame: number; width: number; height: number };
export type StudioNativeTexture = { getVideoFrame(): unknown };
export interface StudioNativeBridge {
  onTexture?(callback: (info: StudioNativeTextureInfo, texture: StudioNativeTexture) => Promise<void> | void): () => void;
  request(session: string, command: StudioNativeCommand, binary?: Uint8Array): Promise<StudioNativeReply>;
  release(session: string): Promise<void>;
  status(): Promise<StudioNativeStatus>;
}
