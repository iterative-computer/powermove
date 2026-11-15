import { useState } from "preact/hooks";
import { TextField, Checkbox, PushButton, token } from "../../src/components";
export function PresetSheet({
  choices,
  save,
  cancel,
  hint = {},
  bindings,
}: {
  choices: any;
  bindings: Record<string, string>;
  save: (v: any) => void;
  cancel: () => void;
  hint?: { effects?: string[]; properties?: string[] };
}) {
  const [name, setName] = useState("Preset");
  const [effects, setEffects] = useState<string[]>(
    hint.effects ??
      (hint.properties?.length ? [] : choices.effects.map((f: any) => f.id)),
  );
  const [properties, setProperties] = useState<string[]>(hint.properties ?? []);
  const [animated, setAnimated] = useState(true);
  const toggle = (values: string[], id: string) =>
    values.includes(id) ? values.filter((v) => v !== id) : [...values, id];
  return (
    <div
      role="dialog"
      focusScope
      aria-modal={true}
      label="Save Animation Preset"
      style={{
        display: "flex",
        flexDirection: "column",
        padding: token("metrics.inset"),
        gap: token("metrics.gap"),
        background: token("colors.panel"),
        color: token("colors.text"),
      }}
    >
      <text role="heading">Save Animation Preset</text>
      <text>Name</text>
      <TextField
        label="Name"
        binding={bindings.name!}
        value={name}
        onInput={(e) => setName(String(e.value ?? ""))}
      />
      <text>Effects</text>
      {choices.effects.map((f: any) => (
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
          }}
        >
          <Checkbox
            label={f.type}
            binding={bindings[`fx.${f.id}`]!}
            value={effects.includes(f.id)}
            onChange={() => setEffects(toggle(effects, f.id))}
          />
          <text>{f.type}</text>
        </div>
      ))}
      <text>Properties</text>
      {choices.properties.map((p: string) => (
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
          }}
        >
          <Checkbox
            label={p}
            binding={bindings[`property.${p}`]!}
            value={properties.includes(p)}
            onChange={() => setProperties(toggle(properties, p))}
          />
          <text>{p}</text>
        </div>
      ))}
      <div
        style={{ display: "flex", flexDirection: "row", alignItems: "center" }}
      >
        <Checkbox
          label="Keep keyframes"
          binding={bindings.animated!}
          value={animated}
          onChange={() => setAnimated(!animated)}
        />
        <text>Keep keyframes</text>
      </div>
      <text>Keyframes are placed relative to the playhead when applied.</text>
      <PushButton label="Cancel" onClick={cancel}>
        Cancel
      </PushButton>
      <PushButton
        label="Save"
        disabled={!name.trim() || !(effects.length + properties.length)}
        onClick={() => save({ name, effects, properties, animated })}
      >
        Save
      </PushButton>
    </div>
  );
}
