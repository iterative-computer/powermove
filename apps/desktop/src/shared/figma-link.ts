/** Leave empty until the owner publishes the plugin and supplies its Community URL. */
export const FIGMA_COMMUNITY_URL = '';
export const FIGMA_PROTOCOL_VERSION = 2;
export type FigmaSetupState = {
  running: boolean;
  paired: boolean;
  pending: { id: string; code: string; expiresAt: number } | null;
  error?: string;
  manifestPath?: string;
};
export type FigmaConnectionStatus = {
  reachable: true;
  tokenValid: boolean;
  compositionOpen: boolean;
  protocolVersion: number;
};
export type FigmaLinkBridge = {
  start(): Promise<FigmaSetupState>;
  closeSetup(): Promise<void>;
  approve(id: string, allow: boolean): Promise<void>;
  stop(): Promise<void>;
  pluginAction(action: 'copy' | 'show' | 'community'): Promise<void>;
  onState(cb: (state: FigmaSetupState) => void): () => void;
  onReceive(cb: (request: { id: string; doc: import('./figma/doc').FigmaDoc }) => void): () => void;
  result(id: string, result: unknown): void;
};
