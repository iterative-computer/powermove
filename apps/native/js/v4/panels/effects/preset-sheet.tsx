import { useState } from "preact/hooks";
import {
  TextField,
  Checkbox,
  PushButton,
  SheetAnchor,
  ToolbarItem,
  token,
} from "../../src/components";
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
  const [name, setName] = useState("");
  const [expanded, setExpanded] = useState<string[]>([]);
  const [effects, setEffects] = useState<string[]>(
    hint.effects ??
      (hint.properties?.length ? [] : choices.effects.map((f: any) => f.id)),
  );
  const [properties, setProperties] = useState<string[]>(hint.properties ?? []);
  const [animated, setAnimated] = useState(true);
  const toggle = (values: string[], id: string) =>
    values.includes(id) ? values.filter((v) => v !== id) : [...values, id];
  return (
    <SheetAnchor
      anchor={0}
      open
      label="Save Animation Preset"
      focusRequest="presets.name"
      preventDefault={{ keydown: ["Enter", "Escape"] }}
      onKeyDown={(e) => {
        if (e.key === "Escape") cancel();
        if (
          e.key === "Enter" &&
          name.trim() &&
          effects.length + properties.length
        )
          save({ name, effects, properties, animated });
      }}
      style={{
        display: "flex",
        flexDirection: "column",
        padding: token("metrics.sheet_inset"),
        gap: token("metrics.gap"),
        background: token("colors.panel"),
        color: token("colors.text"),
      }}
    >
      <text role="heading">Save Animation Preset</text>
      <text>Name</text>
      <TextField
        id="presets.name"
        metadata={{ placeholder: "Preset name" }}
        label="Name"
        binding={bindings.name!}
        value={name}
        onInput={(e) => setName(String(e.value ?? ""))}
      />
      <div
        style={{
          overflowY: "scroll",
          maxHeight: token("metrics.sheet_body_max"),
          flexShrink: 1,
        }}
      >
        <text role="heading">Transform</text>
        {choices.properties.map((p: any) => (
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
            }}
          >
            <Checkbox
              label={p.label}
              binding={bindings[`property.${p.path}`]!}
              value={properties.includes(p.path)}
              onChange={() => setProperties(toggle(properties, p.path))}
            />
            <text>{p.label}</text>
          </div>
        ))}
        {choices.effects.length ? <text role="heading">Effects</text> : null}
        {choices.effects.map((f: any) => (
          <div>
            <div
              style={{
                display: "flex",
                flexDirection: "row",
                alignItems: "center",
              }}
            >
              <ToolbarItem
                label={`${expanded.includes(f.id) ? "Collapse" : "Expand"} ${f.label}`}
                attr={{
                  symbol: expanded.includes(f.id)
                    ? "chevron.down"
                    : "chevron.right",
                }}
                onClick={() => setExpanded(toggle(expanded, f.id))}
              />
              <Checkbox
                label={f.label}
                binding={bindings[`fx.${f.id}`]!}
                value={effects.includes(f.id)}
                onChange={() => setEffects(toggle(effects, f.id))}
              />
              <text>{f.label}</text>
            </div>
            {expanded.includes(f.id)
              ? f.parameters.map((p: any) => (
                  <text
                    style={{
                      paddingLeft: token("metrics.inset"),
                      color: token("colors.secondary"),
                    }}
                  >
                    {p.label}
                  </text>
                ))
              : null}
          </div>
        ))}
      </div>
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
      <div
        style={{
          display: "flex",
          flexDirection: "row",
          justifyContent: "end",
          gap: token("metrics.gap"),
          flexShrink: 0,
        }}
      >
        <PushButton label="Cancel" onClick={cancel}>
          Cancel
        </PushButton>
        <PushButton
          label="Save"
          role="default"
          disabled={!name.trim() || !(effects.length + properties.length)}
          onClick={() => save({ name, effects, properties, animated })}
        >
          Save
        </PushButton>
      </div>
    </SheetAnchor>
  );
}
