# Powermove app icon

`resources/icon/Powermove.icon` is the editable Apple Icon Composer document. Both appearances
use the brand mark geometry from `assets/brand/powermove-light.svg` at the repository root,
with identical size and centering.
The document supplies the graphite/light, silver/dark, and mono treatments.

`resources/icon.icns` is the actual macOS packaging asset, already selected by
`mac.icon` in `electron-builder.yml`. It contains the default/light appearance
at all ten standard sizes, with transparent margins for normal Dock proportions.

After editing the Composer document, regenerate the packaging asset on macOS from `apps/desktop`:

```sh
swift scripts/export-app-icon.swift
```

The script uses Apple's Icon Composer `ictool`, AppKit, and `iconutil`; it does
not modify or flatten the editable source. Set `ICTOOL_PATH` if Icon Composer
is installed in a nonstandard location. `resources/icon/exports/` contains Composer
preview exports; these are not the padded packaging asset.

Builds currently use the static ICNS. Adaptive Liquid Glass app icons require
Xcode 26+ and its `actool` compiler, which the standalone Icon Composer install
does not include. Once that compiler is available, electron-builder supports
setting `mac.icon` to `Powermove.icon` to compile the native appearance catalog.

Replacing the packaging asset takes effect in newly packaged apps; it does not
replace the icon of an already-running Electron development session.
