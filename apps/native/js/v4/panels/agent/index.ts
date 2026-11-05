/** Built-in v4 Agent extension. No app registration or legacy consumer is replaced. */
export { AgentPanel, AgentView, composerEvent } from "./view";
export { HostAgentModel, type AgentRegistration } from "./host";
export type {
  AgentModel,
  Publication,
  Snapshot,
  NativeFacts,
  Action,
} from "./contract";
export { gaps } from "./gaps";
