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
