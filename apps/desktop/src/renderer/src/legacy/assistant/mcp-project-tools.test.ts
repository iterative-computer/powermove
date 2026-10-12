import { describe, expect, it, vi } from 'vitest';
import { mcpProjectTool } from './mcp-project-tools';

const readLocal = vi.hoisted(() => vi.fn());
vi.mock('../core/cloud-media', () => ({ readLocalMediaSource: readLocal }));

function editor(path?: string) {
  return {
    proj: { id: 'P1', name: 'Paper', w: 1920, h: 1080, fps: 30 },
    Export: { busy: false, run: vi.fn(async () => ({ cancelled: false, path: '/exports/Paper.pmv' })) },
    Projects: { put: vi.fn() },
    projectFileState: vi.fn(() => ({ path })),
    saveProject: vi.fn(async () => true),
  };
}

describe('MCP save_project', () => {
  it('saves a project with a file in place, like Command-S', async () => {
    const PM = editor('/Users/me/Paper.pmv');
    await expect(mcpProjectTool(PM as never, '__mcp_save_project', {})).resolves.toEqual({ path: '/Users/me/Paper.pmv', projectId: 'P1' });
    expect(PM.saveProject).toHaveBeenCalledWith();
    expect(PM.Export.run).not.toHaveBeenCalled();
  });

  it('exports a project without a file through the project packer', async () => {
    const PM = editor();
    await expect(mcpProjectTool(PM as never, '__mcp_save_project', {})).resolves.toEqual({ path: '/exports/Paper.pmv', projectId: 'P1' });
    expect(PM.saveProject).not.toHaveBeenCalled();
    expect(PM.Export.run).toHaveBeenCalledWith({ format: 'json', reportPath: true });
  });

  it('reports a failed in-place save', async () => {
    const PM = editor('/Users/me/Paper.pmv');
    PM.saveProject.mockResolvedValue(false);
    await expect(mcpProjectTool(PM as never, '__mcp_save_project', {})).rejects.toThrow('could not be saved');
  });
});

describe('MCP import_media on a desktop host', () => {
  function importer(status = 'created') {
    return {
      proj: { id: 'P1' },
      assetKind: vi.fn(() => 'image'),
      assets: { importBatch: vi.fn(async () => [{ status, persisted: true, asset: { id: 'a1', name: 'shot.png' } }]) },
      bus: { emit: vi.fn() },
      autosave: vi.fn(),
      commandForAsset: vi.fn(() => ({ type: 'add_layer', layerType: 'image', content: { asset: 'a1' } })),
    };
  }

  it('reads the local file and imports it without placing a layer', async () => {
    const file = { name: 'shot.png' };
    readLocal.mockResolvedValueOnce(file);
    const PM = importer();
    await expect(mcpProjectTool(PM as never, '__mcp_import_media', { path: '/tmp/shot.png' })).resolves.toMatchObject({
      assetId: 'a1', status: 'created', persisted: true, addLayer: { layerType: 'image' },
    });
    expect(readLocal).toHaveBeenCalledWith(expect.objectContaining({ name: 'shot.png', sourcePath: '/tmp/shot.png' }), expect.any(Function));
    expect(PM.assets.importBatch).toHaveBeenCalledWith([file]);
  });

  it('rejects relative paths and unreadable files', async () => {
    await expect(mcpProjectTool(importer() as never, '__mcp_import_media', { path: 'shot.png' })).rejects.toThrow();
    readLocal.mockResolvedValueOnce(null);
    await expect(mcpProjectTool(importer() as never, '__mcp_import_media', { path: '/tmp/missing.png' })).rejects.toThrow('not available locally');
  });

  it('reports a failed import', async () => {
    readLocal.mockResolvedValueOnce({ name: 'shot.png' });
    await expect(mcpProjectTool(importer('failed') as never, '__mcp_import_media', { path: '/tmp/shot.png' })).rejects.toThrow('import failed');
  });
});
