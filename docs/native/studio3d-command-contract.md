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
