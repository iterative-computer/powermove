import { useLayoutEffect, useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
import {
  ColorModel,
  type Intent,
  type MenuItem,
  type ParameterRow,
} from "./model";
import {
  CST,
  CST_SECTIONS,
  CURVES,
  EXPORT_CDL,
  IMPORT_CDL,
  LOAD_LUT,
  LUT,
  PANEL_ID,
  PANEL_TITLE,
  WHEELS,
  WHEELS_OF,
  WHEEL_SECTIONS,
  addSlot,
  coverage,
  curveGroup,
  grade,
  move,
  names,
  parameter,
  wheelGroup,
  type GradeEffect,
} from "./contract";

const t = UI.token;
const stop = (e: SurfaceEvent) => e.stopPropagation();

export interface ColorProps {
  controller: ColorModel;
  /** Runs an app command (Load LUT…). */
  command: (name: string) => void;
}

/** What the panel's Add menu offers, in the order a colourist reaches for
 * them; a LUT and a CDL come from a file. */
const ADD_ITEMS: MenuItem[] = [
  { id: WHEELS, label: "Color Wheels" },
  { id: CURVES, label: "Curve" },
  { id: CST, label: "Color Space Transform" },
  null,
  { id: "lut", label: "LUT…" },
  { id: "import-cdl", label: "CDL…" },
];

/** The selected layer's grade, by tool: Color Wheels, Curves, LUT, and any
 * other grading effect (an extension's included). One family with
 * Properties: its header's actions, its layer heading, a disclosure section
 * per grade with the same rows; the wheels and curves are the instruments. */
export function ColorPanel({ controller: c, command }: ColorProps) {
  const [p, update] = useState(c.snapshot());
  useLayoutEffect(() => c.subscribe(() => update(c.snapshot())), [c]);
  // Every grading row projected: the whole list, each tool disclosed. A
  // grade folded here is folded in this panel only.
  useLayoutEffect(() => {
    for (const intent of coverage(p)) c.send(intent);
  }, [p]);
  const [folded, fold] = useState<ReadonlySet<string>>(new Set());
  const editable = p.target === "Layer" && !p.locked;
  const send = (intent: Intent) => {
    if (editable) c.send(intent);
  };
  // Every grade in the order it applies (the effect stack's), each named so
  // two of a kind can be told apart.
  const tools = grade(p);
  const titles = names(tools);
  const firstWheels = tools.find((g) => g.header.definition === WHEELS);
  const slot = addSlot(tools);
  const rows = (list: (ParameterRow | undefined)[]) =>
    list.map(
      (row) =>
        row && <Control key={row.id} row={row} locked={p.locked} model={c} />,
    );
  // Another grade goes after the last one, in one Undo step; a curve starts
  // on Master (its Channel pop-up chooses another).
  const add = (choice: string | null) => {
    if (!editable || !choice) return;
    if (choice === "lut") command(LOAD_LUT);
    else if (choice === "import-cdl") command(IMPORT_CDL);
    else
      send({
        type: "add",
        definition: choice,
        ...(slot === undefined ? {} : { slot }),
        ...(choice === CURVES ? { parameters: { channel: 0 } } : {}),
      });
  };
  const menu = (items: MenuItem[]) =>
    c.platform.contextMenu(items).catch((error) => {
      c.platform.report(error);
      return null;
    });
  // Balance and Tone as Properties' rows (side by side when the panel is
  // wide), then the four wheels.
  const wheelControls = (g: GradeEffect) => [
    <div
      key={`${g.header.id}:balance`}
      role="group"
      label="Balance and Tone"
      style={{ display: "flex", flexDirection: "row", flexWrap: "wrap" }}
    >
      {WHEEL_SECTIONS.slice(0, 2).map((s) => (
        <div
          key={`${g.header.id}:${s.title}`}
          role="group"
          label={s.title}
          style={{
            display: "flex",
            flexDirection: "column",
            flexGrow: 1,
            flexBasis: t("metrics.grade_pair"),
          }}
        >
          <Subheading title={s.title} />
          {rows(s.keys.map((k) => parameter(g, k)))}
        </div>
      ))}
    </div>,
    <Wheels key={`${g.header.id}:wheels`} g={g} locked={p.locked} />,
  ];
  const body = (g: GradeEffect) =>
    g.header.definition === WHEELS
      ? wheelControls(g)
      : g.header.definition === CURVES
        ? [
            ...rows([parameter(g, "channel")]),
            <CurveControl key="curve" g={g} locked={p.locked} />,
          ]
        : g.header.definition === CST
          ? sections(g, CST_SECTIONS)
          : rows(g.parameters);
  // A grade's controls under its parts' names; any row no part names
  // follows them, so nothing is ever left out.
  function sections(
    g: GradeEffect,
    parts: { title: string; keys: string[] }[],
  ) {
    const named = new Set(parts.flatMap((s) => s.keys));
    return [
      ...parts.map((s) => {
        const list = s.keys.map((k) => parameter(g, k)).filter(Boolean);
        return list.length ? (
          <div key={`${g.header.id}:${s.title}`} role="group" label={s.title}>
            <Subheading title={s.title} />
            {rows(list)}
          </div>
        ) : null;
      }),
      ...rows(g.parameters.filter((r) => !named.has(r.parameter.key ?? ""))),
    ];
  }
  return (
    <div
      id={PANEL_ID}
      role="group"
      label={PANEL_TITLE}
      metadata={{ panel: PANEL_ID }}
      style={{
        background: t("colors.panel"),
        borderRadius: t("metrics.panel_radius"),
        display: "flex",
        flexDirection: "column",
      }}
    >
      <Header
        canAdd={editable}
        canExport={p.target === "Layer" && !!firstWheels}
        onAdd={() => void menu(ADD_ITEMS).then(add)}
        onMore={() =>
          void menu([
            { id: "import", label: "Import CDL…", disabled: !editable },
            {
              id: "export",
              label: "Export CDL…",
              disabled: !(p.target === "Layer" && firstWheels),
            },
          ]).then((choice) => {
            if (choice === "import" && editable) command(IMPORT_CDL);
            if (choice === "export" && firstWheels) command(EXPORT_CDL);
          })
        }
      />
      {p.target !== "Layer" ? (
        <UI.EmptyState
          label={p.target === "NoProject" ? "No Composition" : "No Selection"}
          explanation={
            p.target === "NoProject"
              ? "Open a project to start grading."
              : p.target === "NoSelection"
                ? "Select a layer to grade it."
                : "This kind of layer can’t be graded."
          }
          attr={{ symbol: "camera.filters" }}
          action={null}
        />
      ) : (
        <scroll
          id={`${PANEL_ID}:content`}
          role="group"
          label={PANEL_TITLE}
          style={{ display: "flex", flexDirection: "column" }}
        >
          <Heading title={p.name} detail={p.typeLabel} locked={p.locked} />
          {tools.length === 0 ? (
            <Ungraded editable={editable} onAdd={add} />
          ) : null}
          {tools.map((g, i) => {
            const id = g.header.id;
            const title = titles[i];
            const up = move(tools, i, -1);
            const down = move(tools, i, 1);
            return (
              <UI.DisclosureGroup
                key={id}
                id={`${PANEL_ID}:grade:${id}`}
                label={title}
                open={!folded.has(id)}
                onChange={(open) => {
                  const next = new Set(folded);
                  if (open) next.delete(id);
                  else next.add(id);
                  fold(next);
                }}
                // Its order, Reset and Remove, as Effect Controls' header
                // menu offers them.
                onPointerDown={(e: SurfaceEvent) => {
                  if (e.button !== 2) return;
                  stop(e);
                  if (!editable) return;
                  void menu([
                    { id: "up", label: "Move Up", disabled: !up },
                    { id: "down", label: "Move Down", disabled: !down },
                    null,
                    { id: "reset", label: "Reset" },
                    { id: "remove", label: "Remove" },
                  ]).then((choice) => {
                    if (choice === "up" && up) send(up);
                    else if (choice === "down" && down) send(down);
                    else if (choice === "reset" || choice === "remove")
                      send({ type: choice, id });
                  });
                }}
                leading={
                  <UI.Checkbox
                    id={`${PANEL_ID}:on:${id}`}
                    preventDefault={{ click: true }}
                    label={`Enable ${title}`}
                    binding={`${PANEL_ID}:enable:${id}`}
                    value={g.header.enabled}
                    disabled={!editable}
                    onKeyDown={stop}
                    onClick={stop}
                    // A person's click (the AppKit checkbox's action, the
                    // drawn box's click) and the agent's arrive as the box's
                    // change.
                    onChange={(e) => {
                      stop(e);
                      send({ type: "toggle", id });
                    }}
                  />
                }
                accessory={[
                  <GradeAction
                    key="reset"
                    id={`${PANEL_ID}:reset:${id}`}
                    label={`Reset ${title}`}
                    title="Reset"
                    symbol="arrow.counterclockwise"
                    disabled={!editable}
                    onActivate={() => send({ type: "reset", id })}
                  />,
                  <GradeAction
                    key="remove"
                    id={`${PANEL_ID}:remove:${id}`}
                    label={`Remove ${title}`}
                    title="Remove"
                    symbol="minus"
                    disabled={!editable}
                    onActivate={() => send({ type: "remove", id })}
                  />,
                ]}
              >
                {body(g)}
              </UI.DisclosureGroup>
            );
          })}
        </scroll>
      )}
    </div>
  );
}

/** The panel's own header: its title (the tab shows it when tabbed) and its
 * actions at the trailing edge, as Properties' Add Effect: Add a grade, and
 * a CDL's Import and Export. */
function Header({
  canAdd,
  canExport,
  onAdd,
  onMore,
}: {
  canAdd: boolean;
  canExport: boolean;
  onAdd: () => void;
  onMore: () => void;
}) {
  return (
    <div
      id={`${PANEL_ID}:header`}
      data-pm-slot="header"
      style={{
        display: "flex",
        flexDirection: "row",
        alignItems: "center",
        justifyContent: "space-between",
        height: t("metrics.header"),
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.inset"),
      }}
    >
      <text
        style={{
          color: t("colors.text"),
          fontSize: t("typography.title"),
          fontWeight: t("typography.semibold_weight"),
        }}
      >
        {PANEL_TITLE}
      </text>
      <div style={{ display: "flex", flexDirection: "row" }}>
        <UI.ToolbarItem
          id={`${PANEL_ID}:add`}
          label="Add Grade"
          title="Add Grade"
          aria-haspopup="menu"
          disabled={!canAdd}
          metadata={{ symbol: "plus" }}
          onKeyDown={stop}
          onClick={(e) => {
            stop(e);
            if (canAdd) onAdd();
          }}
        />
        <UI.ToolbarItem
          id={`${PANEL_ID}:cdl`}
          label="CDL"
          title="Import or Export CDL"
          aria-haspopup="menu"
          disabled={!canAdd && !canExport}
          metadata={{ symbol: "ellipsis.circle" }}
          onKeyDown={stop}
          onClick={(e) => {
            stop(e);
            if (canAdd || canExport) onMore();
          }}
        />
      </div>
    </div>
  );
}

/** A layer with no grade yet: what can be added, as quiet actions. */
function Ungraded({
  editable,
  onAdd,
}: {
  editable: boolean;
  onAdd: (choice: string) => void;
}) {
  return (
    <div
      role="group"
      label="Add a Grade"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: t("metrics.inner_radius"),
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.inset"),
        paddingTop: t("metrics.gap"),
      }}
    >
      <text
        style={{ color: t("colors.muted"), fontSize: t("typography.minimum") }}
      >
        No grade on this layer.
      </text>
      <div
        style={{
          display: "flex",
          flexDirection: "row",
          flexWrap: "wrap",
        }}
      >
        {ADD_ITEMS.flatMap((i) => {
          if (!i) return [];
          const file = i.id === "lut" || i.id === "import-cdl";
          return [
            <SectionAction
              key={i.id}
              id={`${PANEL_ID}:add:${i.id}`}
              label={
                i.id === "lut"
                  ? "Load LUT…"
                  : i.id === "import-cdl"
                    ? "Import CDL…"
                    : `Add ${i.label}`
              }
              short={i.label}
              symbol={file ? "folder" : "plus"}
              disabled={!editable}
              onActivate={() => onAdd(i.id)}
            />,
          ];
        })}
      </div>
    </div>
  );
}

/** A grade's Reset or Remove: a quiet glyph at its heading's trailing edge. */
function GradeAction({
  id,
  label,
  title,
  symbol,
  disabled,
  onActivate,
}: {
  key?: string;
  id: string;
  label: string;
  title: string;
  symbol: string;
  disabled: boolean;
  onActivate: () => void;
}) {
  return (
    <UI.ToolbarItem
      id={id}
      label={label}
      title={title}
      disabled={disabled}
      attr={{ symbol }}
      onKeyDown={stop}
      onClick={(e) => {
        stop(e);
        if (!disabled) onActivate();
      }}
      style={{
        color: t("colors.muted"),
        symbolSize: t("metrics.symbol_inline"),
      }}
    />
  );
}

/** Lift, Gamma, Gain and Offset: each a drawn wheel over its master and
 * its red, green and blue (as Resolve shows them). The wheels go in two
 * pairs, so a narrow panel shows them two across and a wide one four (never
 * three and one), and every wheel grows with the panel's width. */
function Wheels({
  g,
  locked,
}: {
  key?: string;
  g: GradeEffect;
  locked: boolean;
}) {
  const pairs = [WHEELS_OF.slice(0, 2), WHEELS_OF.slice(2, 4)];
  return (
    <div
      role="group"
      label="Wheels"
      style={{
        display: "flex",
        flexDirection: "row",
        flexWrap: "wrap",
        columnGap: t("metrics.gap"),
        rowGap: t("metrics.gap"),
        paddingTop: t("metrics.gap"),
        paddingBottom: t("metrics.gap"),
        // On the rows' inset.
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.inset"),
      }}
    >
      {pairs.map((pair) => (
        <div
          key={pair[0].key}
          style={{
            display: "flex",
            flexDirection: "row",
            columnGap: t("metrics.gap"),
            flexGrow: 1,
            flexBasis: t("metrics.grade_pair"),
            minWidth: t("metrics.grade_pair"),
          }}
        >
          {pair.map((w) => (
            <Wheel key={w.key} g={g} w={w} locked={locked} />
          ))}
        </div>
      ))}
    </div>
  );
}

/** One instrument: its name, the wheel (its master on the bezel), the
 * master's field and the channels, on one plate. */
function Wheel({
  g,
  w,
  locked,
}: {
  key?: string;
  g: GradeEffect;
  w: (typeof WHEELS_OF)[number];
  locked: boolean;
}) {
  const group = wheelGroup(g, w.key);
  const master = parameter(g, `${w.key}Master`);
  const rgb = (group?.value ?? [w.neutral, w.neutral, w.neutral]) as [
    number,
    number,
    number,
  ];
  // A grade that is off can still be set up before it is turned on.
  const disabled = locked || !group;
  return (
    <div
      role="group"
      label={w.title}
      data-pm-variant="plate"
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "stretch",
        gap: t("metrics.gap"),
        padding: t("metrics.gap"),
        flexGrow: 1,
        flexBasis: t("metrics.grade_wheel"),
      }}
    >
      <text
        style={{
          color: t("colors.secondary"),
          fontSize: t("typography.label"),
          fontWeight: t("typography.semibold_weight"),
          textAlign: "center",
        }}
      >
        {w.title}
      </text>
      {group && (
        <UI.ColorWheel
          id={`${PANEL_ID}:wheel:${g.header.id}:${w.key}`}
          label={`${w.title} colour`}
          binding={group.binding}
          value={rgb}
          neutral={w.neutral}
          min={w.range[0]}
          max={w.range[1]}
          {...(master
            ? {
                master: {
                  value: master.parameter.value as number,
                  min: master.parameter.min ?? w.range[0],
                  max: master.parameter.max ?? w.range[1],
                },
              }
            : {})}
          disabled={disabled}
          onKeyDown={stop}
        />
      )}
      {/* Its numbers are subordinate to it: the master a short field
          centred under the wheel, the channels a caption under that. */}
      {master && (
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            justifyContent: "center",
          }}
        >
          <UI.NumberField
            id={`${PANEL_ID}:field:${master.id}`}
            label={`${w.title} Master`}
            binding={master.parameter.binding}
            value={master.parameter.value as number}
            step={master.parameter.step}
            min={master.parameter.min}
            max={master.parameter.max}
            precision={master.parameter.precision}
            disabled={locked || master.parameter.readonly}
            mixed={master.parameter.mixed}
            onKeyDown={stop}
            onClick={stop}
            style={{
              width: t("metrics.grade_field"),
              textAlign: "center",
            }}
          />
        </div>
      )}
      <div
        role="group"
        label={`${w.title} red ${signed(rgb[0])}, green ${signed(rgb[1])}, blue ${signed(rgb[2])}`}
        style={{
          display: "flex",
          flexDirection: "row",
          justifyContent: "center",
          gap: t("metrics.gap"),
        }}
      >
        {(["channel_red", "channel_green", "channel_blue"] as const).map(
          (ink, i) => (
            <text
              key={ink}
              style={{
                color: t(`colors.${ink}`),
                fontSize: t("typography.caption"),
                fontVariantNumeric: "tabular-nums",
                whiteSpace: "nowrap",
              }}
            >
              {signed(rgb[i])}
            </text>
          ),
        )}
      </div>
    </div>
  );
}

/** A channel value as a colourist reads it: two places, a true minus. */
function signed(v: number) {
  return v.toFixed(2).replace("-", "\u2212");
}

/** One curve's drawn editor, on the panel's inset. */
function CurveControl({
  g,
  locked,
}: {
  key?: string;
  g: GradeEffect;
  locked: boolean;
}) {
  const group = curveGroup(g);
  if (!group) return null;
  const channel = Number(parameter(g, "channel")?.parameter.value ?? 0);
  return (
    <div
      role="group"
      label="Curve"
      style={{
        paddingTop: t("metrics.inner_radius"),
        paddingBottom: t("metrics.gap"),
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.inset"),
      }}
    >
      <UI.CurveEditor
        id={`${PANEL_ID}:curve:${g.header.id}`}
        label="Curve"
        binding={group.binding}
        value={group.value}
        channel={channel}
        disabled={locked}
        onKeyDown={stop}
        style={{ maxWidth: t("metrics.curve_editor_wide") }}
      />
    </div>
  );
}

function Heading({
  title,
  detail,
  locked,
}: {
  title: string;
  detail: string;
  locked: boolean;
}) {
  // As Properties names its layer: the label rank, the kind after the name
  // in the muted ink, a lock when it can't be edited.
  return (
    <div
      role="heading"
      label={detail ? `${title} · ${detail}` : title}
      style={{
        display: "flex",
        flexDirection: "row",
        alignItems: "center",
        gap: t("metrics.inner_radius"),
        height: t("metrics.row"),
        minHeight: t("metrics.row"),
        marginTop: t("metrics.pill_inset"),
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.inset"),
      }}
    >
      <text
        title={title}
        style={{
          color: t("colors.text"),
          fontSize: t("typography.label"),
          fontWeight: t("typography.semibold_weight"),
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
        }}
      >
        {title}
      </text>
      {detail && (
        <text
          style={{
            color: t("colors.muted"),
            fontSize: t("typography.label"),
            whiteSpace: "nowrap",
          }}
        >
          {`· ${detail}`}
        </text>
      )}
      {locked && (
        <text
          attr={{ symbol: "lock" }}
          title="Locked layers can’t be edited."
          style={{
            color: t("colors.muted"),
            symbolSize: t("metrics.symbol_inline"),
          }}
        />
      )}
    </div>
  );
}

/** A part of a grade (Balance, Tone): a step under the grade's heading, on
 * a row of the rows' rhythm. */
function Subheading({ title }: { title: string }) {
  return (
    <text
      role="heading"
      style={{
        display: "flex",
        alignItems: "center",
        height: t("metrics.row"),
        paddingLeft: t("metrics.inset"),
        color: t("colors.muted"),
        fontSize: t("typography.minimum"),
        fontWeight: t("typography.semibold_weight"),
      }}
    >
      {title}
    </text>
  );
}

/** A section's own action: a quiet text action at the trailing edge of its
 * heading (design language, Properties). */
function SectionAction({
  id,
  label,
  short,
  symbol,
  disabled,
  onActivate,
}: {
  key?: string;
  id: string;
  label: string;
  short?: string;
  symbol: string;
  disabled: boolean;
  onActivate: () => void;
}) {
  return (
    <UI.ToolbarItem
      id={id}
      label={label}
      title={label}
      disabled={disabled}
      attr={{ symbol }}
      onKeyDown={stop}
      onClick={(e) => {
        stop(e);
        if (!disabled) onActivate();
      }}
      style={{
        gap: t("metrics.border"),
        paddingLeft: t("metrics.gap"),
        paddingRight: t("metrics.gap"),
        color: t("colors.muted"),
        fontSize: t("typography.label"),
        fontWeight: t("typography.regular_weight"),
        symbolSize: t("metrics.symbol_inline"),
      }}
    >
      {short ?? label}
    </UI.ToolbarItem>
  );
}

/** One control: the parameter's native field with its keyframe button; a
 * right click offers Reset. */
function Control({
  row,
  locked,
  model,
}: {
  key?: string;
  row: ParameterRow;
  locked: boolean;
  model: ColorModel;
}) {
  const p = row.parameter;
  const disabled = locked || p.readonly;
  const common = {
    id: `${PANEL_ID}:field:${row.id}`,
    label: p.label,
    binding: p.binding,
    disabled,
    mixed: p.mixed,
    onKeyDown: stop,
    onClick: stop,
  };
  const field =
    p.control === "toggle" ? (
      <UI.Checkbox {...common} value={p.value as boolean} />
    ) : p.control === "color" ? (
      <UI.ColourWell {...common} value={p.value as string} />
    ) : p.control === "choice" ? (
      <UI.PopUpButton {...common} value={p.value} options={p.options ?? []} />
    ) : p.control === "number" ? (
      <UI.NumberField
        {...common}
        value={p.value as number}
        step={p.step}
        min={p.min}
        max={p.max}
        precision={p.precision}
        unit={p.unit}
      />
    ) : (
      <text
        style={{ color: t("colors.muted"), fontSize: t("typography.body") }}
      >
        {typeof p.value === "string" || typeof p.value === "number"
          ? String(p.value)
          : "Custom value"}
      </text>
    );
  const state = p.keyAtPlayhead
    ? "keyframe at playhead"
    : p.animated
      ? "animated"
      : "not animated";
  return (
    <div
      role="group"
      label={p.label}
      onPointerDown={(e) => {
        if (e.button !== 2) return;
        stop(e);
        void model
          .controlMenu(row)
          .catch((error) => model.platform.report(error));
      }}
    >
      <UI.InspectorRow
        label={p.label}
        metadata={{ parameter: p.id, valueColumn: "panel" }}
        style={{
          color: t(
            locked || !row.enabled ? "colors.muted" : "colors.secondary",
          ),
        }}
        leading={
          <UI.ToolbarItem
            label={`Keyframe, ${p.label}`}
            title={state}
            disabled={disabled}
            aria-pressed={p.keyAtPlayhead}
            attr={{
              symbol: p.keyAtPlayhead
                ? "diamond.fill"
                : p.animated
                  ? "diamond.lefthalf.filled"
                  : "diamond",
              value: state,
            }}
            onClick={(e) => {
              stop(e);
              if (!disabled)
                model.send({
                  type: "parameter",
                  id: p.id,
                  action: e.altKey ? "stopwatch" : "diamond",
                });
            }}
            style={{
              minWidth: t("metrics.hit"),
              minHeight: t("metrics.hit"),
              color: t(p.animated ? "colors.accent" : "colors.muted"),
              symbolSize: t("metrics.symbol_inline"),
            }}
          />
        }
      >
        {field}
      </UI.InspectorRow>
    </div>
  );
}
