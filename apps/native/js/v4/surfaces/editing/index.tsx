/** S034 uses the frozen six-op renderer and existing UI-kit components. */
import { useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { Json, SurfaceEvent } from "../../src/protocol";
import {
  copy,
  presets,
  canStyle,
  menu,
  propertyMenu,
  dispatch,
  type Publication,
  type EditingServices,
  type Animator,
  type Field,
  type ActionBody,
} from "./model";
export * from "./model";
const token = UI.token;
const row = {
  display: "flex",
  flexDirection: "row",
  alignItems: "center",
  height: token("metrics.row"),
  paddingLeft: token("metrics.inset"),
  paddingRight: token("metrics.inset"),
};
const heading = {
  color: token("colors.text"),
  fontSize: token("typography.title"),
  fontWeight: token("typography.semibold_weight"),
};
function SymbolButton({
  id,
  label,
  symbol,
  disabled,
  action,
}: {
  id: string;
  label: string;
  symbol: string;
  disabled?: boolean;
  action: () => void;
}) {
  // Native host resolves the template SF Symbol. Do not substitute text glyphs
  // or invent a shape while the UI kit has no Symbol component.
  return (
    <UI.ToolbarItem
      id={id}
      label={label}
      title={label}
      disabled={disabled}
      attr={{ "data-pm-symbol": symbol }}
      onClick={() => {
        if (!disabled) action();
      }}
    />
  );
}
function FieldControl({
  field,
  disabled,
  write,
}: {
  field: Field;
  disabled: boolean;
  write: (value: Json) => void;
}) {
  const common = {
    label: field.label,
    binding: field.binding,
    disabled: disabled || !field.binding,
    onChange: (e: SurfaceEvent) => {
      if (!disabled && e.value !== undefined) write(e.value);
    },
  };
  if (field.kind === "number" && typeof field.value === "number")
    return (
      <UI.NumberField
        {...common}
        value={field.value}
        unit={field.unit}
        step={field.step}
      />
    );
  if (field.kind === "choice")
    return (
      <UI.PopUpButton
        {...common}
        value={field.value}
        options={field.options ?? []}
      />
    );
  if (field.kind === "colour" && typeof field.value === "string")
    return <UI.ColourWell {...common} value={field.value} />;
  // Unknown authored values stay upstream. No coercion into a writable default.
  return null;
}
function AnimatorGroup({
  a,
  index,
  p,
  services,
}: {
  key?: string;
  a: Animator;
  index: number;
  p: Publication;
  services: EditingServices;
}) {
  const [open, setOpen] = useState(true);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [hover, setHover] = useState<string | null>(null);
  const send = (body: ActionBody) => dispatch(p, services, body);
  const rename = () => {
    const name = renaming?.trim();
    setRenaming(null);
    if (name && name !== a.name) send({ type: "rename", animator: a.id, name });
  };
  const base = `editing/${p.project}/${p.document}/${p.layer}/${a.id}`;
  return (
    <div
      id={base}
      role="group"
      label={`${a.name}, animator ${index + 1} of ${p.animators.length}`}
      onPointerMove={() => setHover(null)}
    >
      <div
        style={row}
        onDragStart={() => send({ type: "begin-reorder", animator: a.id })}
        onDrop={(e) => {
          const data = e.drag?.data;
          if (
            data?.document === p.document &&
            data?.layer === p.layer &&
            typeof data?.animator === "string" &&
            data.animator !== a.id
          )
            send({ type: "reorder", animator: data.animator, before: a.id });
        }}
        attr={{ "data-pm-reorder-id": a.id }}
      >
        <UI.ToolbarItem
          label={a.name}
          attr={{ "data-pm-symbol": open ? "chevron.down" : "chevron.right" }}
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        />
        <UI.Checkbox
          label={`${a.name} enabled`}
          value={a.enabled}
          binding={a.enabledBinding ?? ""}
          disabled={p.locked || !a.enabledBinding}
          onChange={(e: SurfaceEvent) => {
            if (typeof e.value === "boolean")
              send({ type: "enabled", animator: a.id, value: e.value });
          }}
        />
        {renaming !== null ? (
          <UI.TextField
            label={a.name}
            value={renaming}
            binding={a.nameBinding ?? ""}
            disabled={p.locked || !a.nameBinding}
            onInput={(e) => {
              if (typeof e.value === "string") setRenaming(e.value);
            }}
            preventDefault={{ keydown: ["Enter", "Escape"] }}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") rename();
              else if (e.key === "Escape") setRenaming(null);
            }}
            onBlur={rename}
          />
        ) : (
          <text
            style={{
              color: token(a.enabled ? "colors.text" : "colors.muted"),
              fontSize: token("typography.body"),
            }}
            onDoubleClick={() => {
              if (!p.locked) setRenaming(a.name);
            }}
          >
            {a.name}
          </text>
        )}
        {!open && a.animated ? (
          <div
            attr={{ "data-pm-keyframe-state": "animated" }}
            style={{
              width: token("metrics.keyframe"),
              height: token("metrics.keyframe"),
              color: token("colors.accent"),
            }}
          />
        ) : null}
        <SymbolButton
          id={`${base}/menu`}
          label={a.name}
          symbol="ellipsis.circle"
          action={() =>
            services.menu(`${base}/menu`, menu(p, a.id), p, (body) => {
              if (body.type === "begin-rename") {
                if (!p.locked) setRenaming(a.name);
              } else send(body);
            })
          }
        />
      </div>
      {open ? (
        <div>
          <text style={heading}>{copy.range}</text>
          {a.selector.map((field) => (
            <UI.InspectorRow key={field.id} label={field.label}>
              <FieldControl
                field={field}
                disabled={p.locked}
                write={(value) =>
                  send({
                    type: "write",
                    animator: a.id,
                    property: field.id,
                    value,
                  })
                }
              />
              {field.id === "order" && field.value === "random" ? (
                <SymbolButton
                  id={`${base}/shuffle`}
                  label="Shuffle"
                  symbol="arrow.clockwise"
                  disabled={p.locked}
                  action={() => {
                    send({ type: "shuffle", animator: a.id });
                  }}
                />
              ) : null}
            </UI.InspectorRow>
          ))}
          <div style={row}>
            <text style={heading}>{copy.properties}</text>
            <SymbolButton
              id={`${base}/add-property`}
              label={copy.addProperty}
              symbol="plus"
              disabled={p.locked}
              action={() =>
                services.menu(`${base}/add-property`, propertyMenu(a), p, send)
              }
            />
          </div>
          {a.properties.map((field) => (
            <div
              key={field.id}
              onPointerMove={(e) => {
                e.stopPropagation();
                setHover(field.id);
              }}
              onFocus={() => setHover(field.id)}
            >
              <UI.InspectorRow
                label={field.label}
                trailing={
                  hover === field.id ? (
                    <SymbolButton
                      id={`${base}/remove/${field.id}`}
                      label={`Remove ${field.label}`}
                      symbol="minus.circle"
                      disabled={p.locked}
                      action={() => {
                        send({
                          type: "remove-property",
                          animator: a.id,
                          property: field.id,
                        });
                      }}
                    />
                  ) : null
                }
              >
                <FieldControl
                  field={field}
                  disabled={p.locked}
                  write={(value) =>
                    send({
                      type: "write",
                      animator: a.id,
                      property: field.id,
                      value,
                    })
                  }
                />
              </UI.InspectorRow>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
export function TextAnimators({
  publication: p,
  services,
}: {
  publication: Publication;
  services: EditingServices;
}) {
  const base = `editing/${p.project}/${p.document}/${p.layer}`;
  return (
    <div id={base} role="group" label={copy.heading}>
      <div style={row}>
        <text style={heading}>{copy.heading}</text>
        <SymbolButton
          id={`${base}/add`}
          label={copy.add}
          symbol="plus"
          disabled={p.locked}
          action={() =>
            services.menu(
              `${base}/add`,
              presets.map(([preset, label]) => ({
                label,
                action: { type: "preset", preset },
              })),
              p,
              (body) => {
                dispatch(p, services, body);
              },
            )
          }
        />
      </div>
      <UI.ToolbarItem
        label={copy.style}
        disabled={!canStyle(p)}
        title={!canStyle(p) ? copy.selectFirst : copy.style}
        onClick={() => {
          dispatch(p, services, { type: "style" });
        }}
      >
        {copy.style}
      </UI.ToolbarItem>
      {p.animators.length === 0 ? (
        <text
          style={{
            fontSize: token("typography.reading"),
            color: token("colors.muted"),
          }}
        >
          {copy.empty}
        </text>
      ) : (
        p.animators.map((a, index) => (
          <AnimatorGroup
            key={`${p.document}/${p.layer}/${a.id}`}
            a={a}
            index={index}
            p={p}
            services={services}
          />
        ))
      )}
    </div>
  );
}
