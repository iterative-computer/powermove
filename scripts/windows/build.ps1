param()

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
Push-Location $repo
try {
    & bun run --cwd apps/desktop build
    if ($LASTEXITCODE -ne 0) { throw 'Renderer build failed.' }

    # Agent API pack and mod compiler, bundled from target/powermove-resources.
    & (Join-Path $PSScriptRoot 'stage-api-pack.ps1')
    & (Join-Path $PSScriptRoot 'stage-compiler.ps1')

    Push-Location (Join-Path $repo 'crates/powermove-windows')
    try {
        & bunx '@tauri-apps/cli@2.12.1' build --bundles nsis
        if ($LASTEXITCODE -ne 0) { throw 'Tauri bundle build failed.' }
    } finally {
        Pop-Location
    }
    $installers = @(Get-ChildItem (Join-Path $repo 'target/release/bundle/nsis/*-setup.exe'))
    if ($installers.Count -eq 0) { throw 'NSIS installer was not produced.' }
    $installers | ForEach-Object { Write-Host "Installer: $($_.FullName)" }
} finally {
    Pop-Location
}
