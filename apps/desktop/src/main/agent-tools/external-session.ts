import { randomUUID } from 'node:crypto';
import type { WebContents } from 'electron';
import { z } from 'zod';
import type { AgentToolResponseEvent, CodexRunRequest } from '../../shared/ipc';
import { PROJECT_ID } from '../../shared/ipc';
import { PowermoveAgentToolBridge, type PowermoveAgentToolSession } from './bridge';
import { EXTERNAL_MCP_TOOLS } from './external-spec';
import { POWERMOVE_AGENT_TOOLS } from './spec';
import { agentInstructions, agentResultSchema } from '../codex/instructions';
import { prepareAgentWorkspace, discardPartialRun, discardExtensionStage, removeEmptyRunDirectory, type AgentWorkspace, type AgentApiPackFile } from '../codex/workspace';
import { withStageSnapshot, publishExtensionChanges } from '../codex/change-history';
import { validateStagedExtensions } from '../codex/validate-staged-extensions';

const start = z.object({ prompt: z.string().min(1).max(200000), access: z.enum(['project', 'editor']).default('project'), context: z.enum(['project', 'app']).default('project') }).strict();
const changes = z.object({ extensions: z.array(z.object({ id: z.string(), action: z.enum(['created', 'updated', 'removed']), summary: z.string().max(4000) }).strict()).max(100) }).strict();
export interface ExternalSessionOptions {
  bridge: PowermoveAgentToolBridge;
  getOwner(): WebContents | null;
  userData: string;
  extensionsDir: string;
  apiPackFiles(): Promise<AgentApiPackFile[]>;
  attach(request: CodexRunRequest, session: PowermoveAgentToolSession): Promise<() => Promise<void>>;
  refreshExtensions?(ids: string[]): Promise<void>;
  importMedia?(owner: WebContents, args: Record<string, unknown>): Promise<unknown>;
}

/** One external connection owns one private workspace and transaction. */
export class ExternalAgentSession {
  private session: PowermoveAgentToolSession | null = null;
  private workspace: AgentWorkspace | null = null;
  private detach: (() => Promise<void>) | null = null;
  private closing: Promise<unknown> | null = null;
  private disconnected = false;
  private request: CodexRunRequest | null = null;
  private opening: Promise<unknown> | null = null;
  private readonly ownerClosed = () => { void this.close().catch(() => {}); };
  constructor(private readonly options: ExternalSessionOptions) {}

  tools() { return [...EXTERNAL_MCP_TOOLS, ...POWERMOVE_AGENT_TOOLS]; }

  async call(name: string, args: Record<string, unknown>): Promise<unknown> {
    if (this.disconnected) throw new Error('This MCP connection is closed.');
    if (name === 'start_session') {
      if (this.opening) throw new Error('A session is already opening.');
      this.opening = this.open(args).finally(() => { this.opening = null; });
      return this.opening;
    }
    if (name === 'finish_session') {
      const { commit } = z.object({ commit: z.boolean() }).strict().parse(args);
      if (!this.session) throw new Error('No MCP session is active.');
      return this.finish(commit);
    }
    if (name === 'list_projects' || name === 'create_project' || name === 'open_project') {
      if (name !== 'list_projects' && this.session) throw new Error('Finish the current session before changing projects.');
      const owner = this.options.getOwner();
      if (!owner || owner.isDestroyed()) throw new Error('No Powermove editor is available. Use powermove mcp for a hidden host, or open an editor window.');
      // A short-lived inspection session authenticates main-to-renderer control.
      const session = await this.options.bridge.openSession({ runId: `mcp-${randomUUID()}`, owner, baseRevision: 0 });
      try { return await this.renderer(session, `__mcp_${name}`, args); }
      finally { await session.finish(false); }
    }
    const session = this.requireSession();
    if (name === 'publish_extensions') {
      if (session.inspectionOnly) throw new Error('Planning access cannot publish extensions.');
      const input = changes.parse(args), workspace = this.workspace!;
      const changeSet = await withStageSnapshot(workspace, async snapshot => {
        await validateStagedExtensions(snapshot, input.extensions);
        return publishExtensionChanges(snapshot, input.extensions);
      }, input.extensions.map(change => change.id));
      await this.options.refreshExtensions?.(input.extensions.map(change => change.id));
      // Continue from the promoted baseline, with a new run-private stage.
      await discardExtensionStage(workspace);
      this.workspace = await prepareAgentWorkspace(this.request!, this.options.userData, 'project', agentResultSchema(), {
        extensionsDir: this.options.extensionsDir, apiPackFiles: await this.options.apiPackFiles(), workspaceId: `mcp-${session.runId}`
      });
      session.stagingDirectory = this.workspace.stagingDirectory;
      return { changeSetId: changeSet?.id ?? null, extensions: input.extensions, stagingDirectory: this.workspace.stagingDirectory };
    }
    if (name === 'import_media_to_timeline' || (name === 'import_media' && this.options.importMedia)) {
      if (session.inspectionOnly || session.context === 'app') throw new Error('This tool requires project access and an attached project.');
      // Assert the pinned document before host-local import.
      await this.renderer(session, 'get_project_state', { propertyLimit: 1, keyframeLimit: 0 });
      const input = name === 'import_media'
        ? { ...z.object({ path: z.string().min(1).max(4096).refine(value => /^(\/|[A-Za-z]:[\\/])/.test(value) && !value.includes('\0')), folderId: z.string().nullable().optional(), expectedFingerprint: z.string().max(200).optional() }).strict().parse(args), assetOnly: true }
        : z.object({ path: z.string().min(1), at: z.number().finite().min(0).optional() }).strict().parse(args);
      if (!this.options.importMedia) throw new Error('Direct timeline import requires a hidden host. Use import_media and edit_video in the desktop connection.');
      return this.options.importMedia(session.owner, { ...input, projectId: this.request?.projectId });
    }
    const response = await this.options.bridge.callTool(session, name, args, this.workspace?.root);
    return response;
  }

  private requireSession() {
    if (!this.session || this.closing) throw new Error('Call start_session before using Powermove tools.');
    return this.session;
  }

  private async renderer(session: PowermoveAgentToolSession, name: string, args: Record<string, unknown>) {
    const response = await this.options.bridge.callRenderer(session, name, args);
    if (!response.ok) throw new Error(response.error ?? 'Powermove tool failed.');
    const content = response.content.find(item => item.type === 'text');
    return content?.type === 'text' ? JSON.parse(content.text) : null;
  }

  private async open(args: Record<string, unknown>) {
    if (this.session || this.closing) throw new Error('Finish the current MCP session first.');
    const input = start.parse(args);
    if (input.context === 'app' && input.access === 'editor') throw new Error('App sessions require project access.');
    const owner = this.options.getOwner();
    if (!owner || owner.isDestroyed()) throw new Error('No Powermove editor is available. Use powermove mcp for a hidden host.');
    const session = await this.options.bridge.openSession({ runId: `mcp-${randomUUID()}`, owner, baseRevision: 0, context: input.context, inspectionOnly: input.access === 'editor' });
    this.session = session;
    owner.once('destroyed', this.ownerClosed);
    owner.once('render-process-gone', this.ownerClosed);
    try {
      const snapshot = await this.renderer(session, '__mcp_context', {});
      if (input.context === 'project' && (snapshot.home || !PROJECT_ID.test(snapshot.projectId))) throw new Error('Create or open a project before starting a project session.');
      const request: CodexRunRequest = {
        id: session.runId, threadId: `mcp-thread-${randomUUID()}`, provider: 'chatgpt',
        mode: input.access === 'editor' ? 'editor' : 'autonomous', access: input.access, context: input.context,
        prompt: input.prompt, projectId: input.context === 'app' ? 'powermove-global' : snapshot.projectId,
        projectName: input.context === 'app' ? 'Powermove' : snapshot.projectName,
        projectJSON: input.context === 'app' ? '{}' : snapshot.projectJSON,
        schema: null, images: [], attachments: [], model: null, reasoningEffort: null, consentToken: null
      };
      this.request = request;
      if (input.access !== 'editor') {
        this.workspace = await prepareAgentWorkspace(request, this.options.userData, 'project', agentResultSchema(), {
          extensionsDir: this.options.extensionsDir, apiPackFiles: await this.options.apiPackFiles(), workspaceId: `mcp-${session.runId}`
        });
        session.stagingDirectory = this.workspace.stagingDirectory;
      }
      this.detach = await this.options.attach(request, session);
      if (this.disconnected) throw new Error('MCP disconnected while opening its session.');
      return { runId: session.runId, projectId: request.projectId, access: request.access,
        ...(this.workspace ? { workspace: this.workspace.root, stagingDirectory: this.workspace.stagingDirectory,
          apiPackDirectory: this.workspace.apiPackDirectory, artifactsDirectory: this.workspace.runDirectory,
          instructions: agentInstructions({ projectName: request.projectName, artifactPath: this.workspace.runDirectory, access: 'project', context: input.context, extensionsDir: this.workspace.stagingDirectory }) } : {})
      };
    } catch (error) { await this.finish(false); throw error; }
  }

  private finish(commit: boolean): Promise<unknown> {
    if (this.closing) return this.closing;
    this.closing = (async () => {
      try {
        try { await this.detach?.(); }
        finally { if (this.session) {
          this.session.owner.removeListener('destroyed', this.ownerClosed);
          this.session.owner.removeListener('render-process-gone', this.ownerClosed);
        } }
        return await this.session?.finish(commit) ?? { changed: false };
      }
      finally {
        try {
          if (this.session && !this.session.isClosed()) await this.session.finish(false);
          if (this.workspace) {
            if (commit) { await discardExtensionStage(this.workspace); await removeEmptyRunDirectory(this.workspace); }
            else await discardPartialRun(this.workspace);
          }
        } finally { this.detach = null; this.session = null; this.workspace = null; this.request = null; }
      }
    })().finally(() => { this.closing = null; });
    return this.closing;
  }

  async close(): Promise<void> {
    this.disconnected = true;
    await this.opening?.catch(() => {});
    await this.finish(false);
  }
}

/** Keep binary images and actual tool errors intact over the public MCP. */
export function mcpToolResult(value: unknown) {
  if (value && typeof value === 'object' && 'content' in value && 'ok' in value) {
    const response = value as AgentToolResponseEvent;
    const content = response.content.map(item => item.type === 'image' ? { ...item, data: Buffer.from(item.data).toString('base64') } : item);
    if (!response.ok && response.error) content.push({ type: 'text', text: response.error });
    return { content, isError: !response.ok };
  }
  return { content: [{ type: 'text', text: JSON.stringify(value ?? {}) }], isError: false };
}
