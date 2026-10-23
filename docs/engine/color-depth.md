# Project color depth

`project.colorDepth` is the single saved working precision: 8-bit normalized,
16-bit float, or 32-bit float. The Project Settings Composition card owns the
Color depth row and writes it through `set_composition`, so each change is one
Undo step. The native-only Color card retains the linear-light switch; it has
no second precision control.

Older native-engine projects may have saved `project.color.precision` without
`colorDepth`. On file hydration, legacy deserialization, and typed snapshot
sanitization, TypeScript maps `float32` to depth 32 and `float16` to depth 16,
then removes the old precision field. An explicit `colorDepth` of 8, 16, or 32
wins. Projects with neither setting open at depth 16. Migration copies the
project and color object, so callers' source documents are preserved.

The protocol's `ProjectColor.precision` remains a derived transport value for
the native engine: depths 8, 16, and 32 map to Unorm8, Float16, and Float32.
The native renderer selects its working format from the same `colorDepth`
setting. A 32-bit depth alone
does not require a native-engine stamp; linear blending and other native-only
features still do.
