# Powermove app icon

`resources/icon/Powermove.icon` is the editable Apple Icon Composer document. Both appearances
use the brand mark geometry from `assets/brand/powermove-light.svg` at the repository root,
with identical size and centering.
The document supplies the graphite/light, silver/dark, and mono treatments.

`resources/icon/Powermove.icon` is the macOS packaging asset selected by
`mac.icon` in `electron-builder.yml`. electron-builder compiles it with Apple's
`actool`, allowing macOS to select light, dark, tinted, and clear renditions.
`resources/icon.icns` remains as a legacy fallback for tooling that cannot compile
Icon Composer catalogs.

After editing the Composer document, regenerate the packaging asset on macOS from `apps/desktop`:

```sh
swift scripts/export-app-icon.swift
```

The script uses Apple's Icon Composer `ictool`, AppKit, and `iconutil`; it does
not modify or flatten the editable source. Set `ICTOOL_PATH` if Icon Composer
is installed in a nonstandard location. `resources/icon/exports/` contains Composer
preview exports; these are not the padded packaging asset.

Adaptive Liquid Glass app icons require Xcode 26+ and its `actool` compiler. On
release machines, select the full Xcode developer directory with `xcode-select`
before running the macOS build.

Replacing the packaging asset takes effect in newly packaged apps; it does not
replace the icon of an already-running Electron development session.
