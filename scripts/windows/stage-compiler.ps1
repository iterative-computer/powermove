param(
    # Defaults to the dev resources folder the host reads as `resources_dir/compiler`.
    [string]$Destination
)

# Stages the mod compiler the Rust host runs without Node.js:
#   esbuild.exe            bundler, driven over its CLI (same version as apps/desktop)
#   svelte-compiler.js     Svelte's prebuilt UMD compiler, run in embedded QuickJS
#   runtime-exports.json   export names of every runtime module a bundle may import
#   LICENSE-esbuild.md, LICENSE-svelte.md
# See crates/powermove-host/src/extensions/compiler.rs.

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$desktop = Join-Path $repo 'apps/desktop'
if (-not $Destination) { $Destination = Join-Path $repo 'target/powermove-resources/compiler' }

$esbuildPackage = (Resolve-Path (Join-Path $desktop 'node_modules/esbuild')).Path
$esbuildVersion = (Get-Content (Join-Path $esbuildPackage 'package.json') -Raw | ConvertFrom-Json).version
# bun keeps each package in node_modules/.bun/<name>@<version>; the platform binary sits beside esbuild.
$esbuildExe = Join-Path $repo "node_modules/.bun/@esbuild+win32-x64@$esbuildVersion/node_modules/@esbuild/win32-x64/esbuild.exe"
if (-not (Test-Path $esbuildExe)) { throw "esbuild $esbuildVersion for win32-x64 is not installed: $esbuildExe" }

$sveltePackage = (Resolve-Path (Join-Path $desktop 'node_modules/svelte')).Path
$svelteCompiler = Join-Path $sveltePackage 'compiler/index.js'
if (-not (Test-Path $svelteCompiler)) { throw "Svelte's prebuilt compiler is missing: $svelteCompiler" }

New-Item -ItemType Directory -Force $Destination | Out-Null
Copy-Item $esbuildExe (Join-Path $Destination 'esbuild.exe') -Force
Copy-Item $svelteCompiler (Join-Path $Destination 'svelte-compiler.js') -Force
Copy-Item (Join-Path $esbuildPackage 'LICENSE.md') (Join-Path $Destination 'LICENSE-esbuild.md') -Force
Copy-Item (Join-Path $sveltePackage 'LICENSE.md') (Join-Path $Destination 'LICENSE-svelte.md') -Force

# Lists each namespace's names the way compiler.ts does (Object.keys of the module).
$script = @'
import { EDITOR_HELPER_EXPORTS, SVELTE_RUNTIME_MODULES } from './src/shared/extension-runtime.ts';
const svelte = JSON.parse(await Bun.file('./node_modules/svelte/package.json').text());
const esbuild = JSON.parse(await Bun.file('./node_modules/esbuild/package.json').text());
const modules = { powermove: [...EDITOR_HELPER_EXPORTS] };
for (const specifier of SVELTE_RUNTIME_MODULES) modules[specifier] = Object.keys(await import(specifier));
await Bun.write(process.argv[2], JSON.stringify({ svelte: svelte.version, esbuild: esbuild.version, modules }, null, 2) + '\n');
'@
$generator = Join-Path $desktop '.stage-compiler-exports.ts'
Set-Content -Path $generator -Value $script -Encoding utf8
Push-Location $desktop
try {
    & bun run $generator (Join-Path $Destination 'runtime-exports.json')
    if ($LASTEXITCODE -ne 0) { throw 'Listing runtime exports failed.' }
} finally {
    Pop-Location
    Remove-Item $generator -Force -ErrorAction SilentlyContinue
}

Write-Host "Mod compiler staged in $Destination (esbuild $esbuildVersion)"
