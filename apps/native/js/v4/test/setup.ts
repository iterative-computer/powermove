import { nativeWire } from "../eval/native-wire";

// A plain `bun test` exercises native conformance too; never silently skip it.
if (!process.env.PM_SURFACE_WIRE || !process.env.PM_UI_KIT_WIRE) {
  const wire = nativeWire();
  process.env.PM_SURFACE_WIRE ??= wire.surface;
  process.env.PM_UI_KIT_WIRE ??= wire.kit;
}
