export type AgentClientId = 'claude-code' | 'codex' | 'claude-desktop' | 'gemini-cli' | 'cursor' | 'vscode' | 'windsurf' | 'lm-studio' | 'opencode';

export interface AgentClientStatus {
  id: AgentClientId;
  name: string;
  /** Whose plan it runs on, in a few words. */
  detail: string;
  /** `connected`: Powermove is in its MCP settings. `available`: installed, not added yet. */
  state: 'connected' | 'available' | 'not-installed' | 'unreadable';
  /** The entry was set up by hand; Powermove leaves it alone. */
  custom: boolean;
  configPath: string;
}

export interface AgentClientsInfo {
  clients: AgentClientStatus[];
  /** The command any MCP-capable agent runs to reach Powermove. */
  command: string;
  args: string[];
}
