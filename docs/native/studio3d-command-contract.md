# Native Studio command behavior

## World lighting and the fallback sun

An active World in `hdri` or `sky` mode supplies the scene lighting. Scene
assembly does not add the fallback sun when that World is present, even if
its background visibility is off or its intensity is zero. Explicit light
layers are unchanged.

Scenes without an active lighting World retain the existing fallback sun.
A hidden World, an expired World, or a World in `none` mode does not suppress
that fallback. This requires no project migration or new stored property.
Existing World-only scenes lose the previously invisible extra sun; legacy
scenes without World lighting keep their appearance.

## Geometry commands without a window

`auto_studio`, `drop_to_floor`, `place_on`, and `explode` request model
geometry from the native media host at the command's immutable project
snapshot. They use the same approved original files and
`pm_scene3d::render::Resource::load` decoder as Studio rendering. Model poses
are measured at the requested local time. No editor view or earlier render
is needed, and a missing original remains an explicit error rather than a
guessed box. Hosts can still inject `Automation::scene3d_assets` explicitly.

`light_highlight` accepts a world-space point and surface normal directly;
that form does not require an editor view or a geometry pick.

## Library and project lock durability

Lock ownership is written to a sibling `.owner` record with exclusive temp
creation, write, file sync, rename, and directory sync. A failed write leaves
the last complete record intact. The advisory lock keeps its stable inode:
renaming that inode could allow two processes to hold different locks for
the same library. The original file retains a best-effort compatibility
mirror for earlier native builds.

After acquiring the advisory lock on a local volume, an unreadable or empty
legacy lock record can be recovered. Another process's live advisory lock
still prevents recovery; valid fresh owner records remain protected. An
unreadable record on a shared volume is still treated conservatively.

## Headless connection discovery

Headless startup probes `profile/mcp/connection.json` before selecting proxy
mode. A dead PID, refused loopback endpoint, or unusable descriptor is
archived privately under the profile and removed from discovery, using the
same publication lock as server startup. Startup can then create a headless
host normally. Live or uncertain owners are preserved. A normal server
shutdown still removes only its own descriptor.

## Explicit recovery over MCP

A normal headless EOF closes library projects durably and retires their
journals. An unsaved named working file is deferred instead: EOF never
silently overwrites it. A proxy disconnect only ends that client session;
it does not close the application's projects.

If interrupted work needs a decision, `open_project` returns
`recoveryRequired: true`, a `ticket`, and the available `recovery.choices`.
It does not change the active project or accept recovery implicitly. For
example, after inspecting that response:

```json
{
  "projectId": "project-id",
  "recovery": {"ticket": "returned-ticket", "decision": "recover"}
}
```

Pass that object to `open_project`. Available decisions are `recover`,
`discard` (open the saved base and archive the rejected journal), and
`open_as_copy`. An unrecoverable journal instead offers
`open_saved_version`, preserving the rejected files. Tickets are bound to
one project and consumed once. The kernel still checks whether the
recovery inputs changed before accepting a choice.

`list_projects.pendingRecoveries` also exposes pending opens, including
a recovery encountered while opening a launch-file argument before MCP
connects. Existing `open_project` calls need no new arguments when there
is no interrupted work.

## Glass highlights above the studio floor

With the studio floor enabled, `light_highlight` keeps the reflected ray and
shortens the light's distance when necessary to fit the softbox above Y = 0.
An automatic softbox size follows that adjusted distance. An explicit size
is preserved. If no above-floor placement exists for that point and size,
the command fails without adding a light, explaining that a higher surface
point or smaller softbox is needed. Scenes without a studio floor retain
the unrestricted reflection placement.

## Original glTF/GLB scale

`edit_3d` can add an imported model with its metre dimensions and shared
source origin:

```json
{
  "operation": "add_object",
  "id": "product-part",
  "object": {"source": {"assetId": "imported-model-id"}},
  "keepRealWorldScale": true
}
```

This is opt-in. Omit it or pass `false` to retain the existing independent
unit-size fit. The command reads the approved model's original fit and
inverts it using the active World's `metersPerUnit` (default `0.1`). Separate
files therefore retain their relative dimensions and their common origin.
No geometry, renderer behavior, or existing project data is changed.

The resulting position and scale are ordinary, visible layer properties;
`get_3d_scene` returns them. Add with this option before applying further
object transforms. A later scale edit replaces the absolute layer factor.
Other formats are rejected for this option because their metre units are
not reliably declared. Normal imports of those formats remain supported.

## Scale units at the command boundary

The existing two representations remain explicit for data compatibility:

| Address | Unit | Example |
| --- | --- | --- |
| `edit_3d` object `p.sx`, `p.sy`, `p.sz` | Factor | `1.188` means 118.8% |
| `apply_commands` `scale.x`, `scale.y`, `scale.z`, including keyframes | Percent | `118.8` means 118.8% |
| `get_3d_scene` object `p.sx`, `p.sy`, `p.sz` | Factor | `1.188` |

Both paths are validated and regression-tested against the same evaluated
transform. Object scale channels are converted to percentages when stored,
including their keys; readback converts them to factors. Unsupported property
aliases and strings such as `"118.8%"` are rejected. Numeric values are never
guessed or silently reinterpreted between these units.
