param(
    # Defaults to the dev resources folder the debug host reads (config.resources_dir).
    [string]$Destination
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
if (-not $Destination) { $Destination = Join-Path $repo 'target/powermove-resources' }
$desktop = Join-Path $repo 'apps/desktop'

# The agent API pack (mirrors extraResources api-pack in apps/desktop/electron-builder.yml).
$files = [ordered]@{
    'EXTENSIONS.md'            = 'docs/EXTENSIONS.md'
    'SCENE3D.md'               = 'docs/SCENE3D.md'
    'blender-schema.ts'        = 'src/shared/blender.ts'
    'scene3d-materials.ts'     = 'src/renderer/src/core/scene3d/materials.ts'
    'scene3d-modeling.ts'      = 'src/renderer/src/core/scene3d/modeling.ts'
    'scene3d-schema.ts'        = 'src/renderer/src/core/scene3d/schema.ts'
    'scene3d-layers.ts'        = 'src/renderer/src/core/scene3d/layers.ts'
    'scene3d-operations.ts'    = 'src/renderer/src/core/scene3d/operations.ts'
    'BACKGROUND_TESTING.md'    = 'docs/background-testing.md'
    'api.ts'                   = 'src/renderer/src/kernel/api.ts'
    'extensions.ts'            = '../../packages/registry/src/manifest.ts'
    'project.ts'               = 'src/renderer/src/core/types/project.ts'
    'commands.ts'              = 'src/renderer/src/core/types/commands.ts'
}
$samples = @('gradient-tint', 'media-browser')

$pack = Join-Path $Destination 'api-pack'
if (Test-Path $pack) { Remove-Item -Recurse -Force $pack }
New-Item -ItemType Directory -Force $pack | Out-Null
foreach ($name in $files.Keys) {
    $source = Join-Path $desktop $files[$name]
    if (-not (Test-Path $source)) { throw "API pack source missing: $source" }
    Copy-Item $source (Join-Path $pack $name)
}
foreach ($sample in $samples) {
    $target = Join-Path $pack "samples/$sample"
    New-Item -ItemType Directory -Force $target | Out-Null
    Copy-Item -Recurse (Join-Path $desktop "docs/samples/$sample/*") $target
}

# Prompt text and the editor tool catalog, generated from the TypeScript sources.
& bun (Join-Path $PSScriptRoot 'agent-prompts.ts') (Join-Path $Destination 'agent/prompts.json')
if ($LASTEXITCODE -ne 0) { throw 'Generating agent prompts failed.' }
Write-Host "Staged the agent API pack in $pack"
