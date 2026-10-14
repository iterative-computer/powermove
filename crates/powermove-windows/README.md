# Powermove for Windows

Requires Windows 11, Rust stable with the MSVC build tools, bun, and WebView2.
Run these commands in `cmd.exe` from the repository root:

```bat
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
bun run --cwd apps/desktop build
cargo run -p powermove-windows
```

Build the release executable and per-user NSIS installer:

```bat
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/windows/build.ps1
```

The installer is written to `target/release/bundle/nsis/`. The script uses
`bunx @tauri-apps/cli@2.12.1`; no global Tauri CLI installation is needed.
The app and installer version come from `workspace.package.version`.

Debug builds load the renderer, built-in manifests, and ffmpeg from the
checkout. Release builds load them from the executable's resource directory.
When adding a built-in extension, add its manifest mapping in `tauri.conf.json`.
Only manifests are bundled; extension implementations are compiled into the UI.

The profile is `%APPDATA%\Powermove`, scratch files use `%TEMP%\Powermove`,
and logs append to `%APPDATA%\Powermove\logs\powermove.log`.
Logs omit authentication queries and project IDs. Debug builds also log to stderr.
The host store is flushed when the last window closes or the app exits.

The installer registers `.pmv` as **Powermove Project**. Opening one from
Explorer (or a second launch with a path) opens it in the most recent editor
window; a file passed at startup waits until the window is ready.

Windows icons were generated from
`apps/desktop/resources/icon/exports/Powermove-iOS-Dark-1024x1024@1x.png`:

```bat
bunx @tauri-apps/cli@2.12.1 icon "apps/desktop/resources/icon/exports/Powermove-iOS-Dark-1024x1024@1x.png" --output crates/powermove-windows/icons
```
