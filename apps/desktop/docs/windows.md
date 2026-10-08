# Windows x64

Powermove builds a native Windows desktop app for Intel and AMD PCs. The same
editor, extensions, timeline, GPU preview, project files, agents, cloud account,
and MP4/ProRes export run on both desktop platforms. Windows ARM and 32-bit
builds are outside this port.

## Run from source

Install Node.js 22 or newer and bun 1.3.14. In PowerShell, from the repository:

```powershell
bun install --frozen-lockfile
bun run dev
```

The Windows agents and FFmpeg install automatically; Xcode and the Mac haptics
addon are not needed. Blender features use an installed Windows Blender, or
the executable selected in Settings.

Ctrl replaces Command for editor shortcuts. The title bar uses Windows' native
minimize, maximize, and close buttons. `.pmv` projects retain their existing
format and can be opened on either platform. Alignment haptics are available
only on supported Mac hardware.

## Agent access

ChatGPT uses the bundled native Codex Windows sandbox. Windows Project commands
for other providers also use that sandbox; Claude's Project process is enclosed
by it because Claude does not provide native Windows sandboxing. The private
Claude runtime directory is writable for its login and session state. Sandbox
startup failure stops the run; it never retries with Computer access.

Codex supports elevated and unelevated Windows sandboxes. Initial elevated
setup can require Windows administrator approval. An explicit user choice in
the Codex `[windows]` configuration is retained. Computer access still requires
the existing per-run confirmation. See the [Windows sandbox documentation](https://learn.chatgpt.com/docs/windows/windows-sandbox).

## Build and validate

```powershell
bun run --cwd apps/cloud types
bun run --cwd apps/desktop typecheck
bun run --cwd apps/desktop test -- src/main/windows-native.test.ts src/main/platform.test.ts src/renderer/src/platform.test.ts
bun run dist:win
```

The NSIS installer is written to `apps/desktop/dist/Powermove-<version>-win-x64.exe`.
It includes the encoder, Codex and its companion tools, Claude, built-in
extensions, and extension compiler. Packaging checks the native executables
and applies the same Electron fuse policy as the Mac app.

`dist:win` explicitly disables publishing. Local/test installers are unsigned;
Windows can show a publisher warning. Distribution signing and a Windows
release require separate authorization and private signing configuration.

`.github/workflows/windows-validation.yml` type-checks, tests native Windows
operations and real Electron save/import/export flows, builds both desktop and
CLI, and retains a test installer for seven days. It has read-only repository
permissions and does not publish releases or deploy services.

## Command-line host

```powershell
npx powermove-cli@latest serve
npx powermove-cli@latest install
npx powermove-cli status
npx powermove-cli logs
npx powermove-cli uninstall
```

Use a package version containing this port; the source changes do not update
the already-published npm package. From this checkout, `bun run serve` builds
and starts the current host.

Windows `install` registers a normal-user Task Scheduler task that starts at
login. The hidden host restarts after crashes or self-updates. Uninstall removes
the task without deleting projects, saved credentials, or profile data.
