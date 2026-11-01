import { createRoot } from "../index";
import { InspectorRow, NumberField, PopUpButton } from "./index";
const root = createRoot(async (c) => ({
  accepted: true,
  revision: c.baseRevision + 1,
}));
(globalThis as Record<string, unknown>).kitProbe = (async () => {
  await root.render(
    <InspectorRow label="Opacity">
      <NumberField value={50} binding="opacity" unit="%" />
      <PopUpButton
        label="Quality"
        value={null}
        binding="quality"
        options={[{ v: null, label: "Auto" }]}
      />
    </InspectorRow>,
  );
  const result = {
    revision: root.inspect().revision,
    components: [...root.host.nodes.values()].filter(
      (n) => n.props["data-pm-component"],
    ).length,
    process: typeof process,
    document: typeof document,
    window: typeof window,
    buffer: typeof Buffer,
  };
  root.dispose();
  return result;
})();
