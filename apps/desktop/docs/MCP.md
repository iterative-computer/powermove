# Powermove MCP

Agents can use Powermove through MCP, including creating and exporting a video without opening the desktop app. The server uses the same editor, tool definitions, media pipeline, extension validation and Undo history as Powermove's built-in agents.

## Built-in agents

Connecting ChatGPT, Claude or an API-compatible provider in Powermove automatically gives its agent the Powermove tools. No MCP settings or separate installation are needed. Project editing runs can import media, build and review a video, save an editable `.pmv` and export MP4 or ProRes with audio. Desktop agent deliveries use unique filenames in `~/Powermove` without a file picker. Planning runs remain read-only. API models must support function calling.

The configuration below is only for agents running outside Powermove.

## Create videos without opening the app

Build the CLI from this checkout (`bun install`, then `bun run build:cli`). Install its hidden renderer once:

```sh
node packages/cli/bin/powermove.mjs mcp-install-browser
```

Add this stdio MCP server to your agent's configuration, replacing the path with your checkout or locally installed CLI:

```json
{
  "mcpServers": {
    "powermove": {
      "command": "node",
      "args": ["/absolute/path/to/packages/cli/bin/powermove.mjs", "mcp"]
    }
  }
}
```

The MCP client starts and stops the host. Chromium runs invisibly; no Powermove window or agent-provider login is required for editing and export. The default profile is `~/.powermove-headless`, separate from desktop projects. Exports go to `~/Powermove`. Use `--user-data /absolute/profile` and `--exports /absolute/output` to choose other folders. Concurrent independent hosts need different profiles.

## Connect to the desktop app

On macOS, launching the installed Powermove app automatically adds its connection to Claude Desktop when Claude is installed on this Mac. Existing Claude connections and settings are preserved. If Claude was already running, restart Claude once to load the new connection. Removing or customizing the connection in Claude is respected on later Powermove launches. Development and isolated test launches do not change Claude settings.

For other MCP clients, open Powermove and add:

```json
{
  "mcpServers": {
    "powermove": {
      "command": "/Applications/Powermove.app/Contents/MacOS/Powermove",
      "args": ["--powermove-mcp"]
    }
  }
}
```

This connects to the running app without opening a second editor. A CLI can also connect to a running desktop or `serve` host with `mcp --connect --user-data /absolute/profile`. On macOS, the desktop profile is normally `~/Library/Application Support/Powermove`.

## Agent workflow

1. `list_projects`, `create_project`, or `open_project` selects the project.
2. `start_session` with the user's task returns a private workspace, staged extensions, API documentation and harness instructions. `access: "editor"` creates a planning session that can only inspect.
3. `import_media` imports host-local footage/audio/images. `apply_commands`, `edit_video` and `edit_3d` build the composition. Use `get_project_state` for IDs and channels, source-media tools to inspect footage, and `render_frames`/`get_workspace_state` to verify the result.
4. If extensions are needed, edit their private staging directory and call `publish_extensions`. It compiles, scans and promotes them with the normal validation. The returned staging path is the new baseline. Verify the loaded extension before continuing.
5. `save_project` writes an editable `.pmv` including its media. `export_video` writes MP4 or ProRes with audio and returns the actual saved path after encoding completes.
6. `finish_session` with `commit: true` keeps edits; `false` attempts a safe rollback. Already saved files and locally published extensions remain. Disconnecting cancels child tasks and attempts to roll back unfinished composition edits. Panel actions and interleaved edits retain normal editor Undo and may produce a rollback warning.

`import_media` imports durable media in both desktop and hidden hosts; use `apply_commands` or `edit_video` to place its returned asset ID. Hidden hosts also offer `import_media_to_timeline` for direct import and placement.

All filesystem paths refer to the host machine. A remote MCP client must use its own host-file tooling to put inputs there and retrieve outputs. Shell and web tools belong to the calling agent, and are not replaced by this MCP.

Delegation uses Powermove's connected providers and model catalog. External clients use `mode: "wait"` or `task_status` to receive child results; Powermove cannot automatically resume a foreign agent. Thread watches likewise require `mode: "wait"` or `thread_read`. Closing the MCP session cancels its child tasks. Independent threads launched by `thread_launch` keep their normal lifecycle.

Desktop connections retain the app's configured Store and transcription features. The hidden host inherits `serve` capabilities: the Store is unavailable and transcription needs a configured model/service; 3D features that use Blender need Blender installed. It declines confirmation dialogs rather than silently approving them. Renderer and encoder failures are returned as tool errors.

## Connection and protocol

The server speaks newline-delimited JSON-RPC over stdio. Supported handshake versions are `2025-11-25`, `2025-06-18`, `2025-03-26` and `2024-11-05`; unknown versions negotiate the newest supported version. Diagnostics use stderr. Tool failures use MCP `isError` results, and frame/panel images remain MCP image content.

Local host discovery uses a random credential in `<profile>/mcp/connection.json`, with owner-only permissions on Unix, and an authenticated loopback socket. Credentials are never placed in agent configuration. The descriptor is removed on clean shutdown and a stale descriptor does not grant a session. Keep the profile outside Git. This grants project editing in that local profile; agents should only connect under the user's instruction.

To verify the complete workflow from source:

```sh
node apps/desktop/scripts/test-mcp.mjs
```

It uses a temporary profile and checks actual video output, audio, project saving, rollback, planning restrictions and shutdown, without touching the live app's profile.
