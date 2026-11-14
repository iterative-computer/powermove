import { useLayoutEffect, useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
import { ColorModel, type Intent, type ParameterRow } from "./model";
import {
  CST,
  CURVES,
  CURVE_CHANNELS,
  EXPORT_CDL,
  IMPORT_CDL,
  LOAD_LUT,
  LUT,
  PANEL_ID,
  PANEL_TITLE,
  WHEELS,
  WHEELS_OF,
  WHEEL_SECTIONS,
  coverage,
  curveGroup,
  grade,
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

/** The selected layer's grade, by tool: Color Wheels, Curves, LUT, and any
 * other grading effect (an extension's included). */
export function ColorPanel({ controller: c, command }: ColorProps) {
  const [p, update] = useState(c.snapshot());
  useLayoutEffect(() => c.subscribe(() => update(c.snapshot())), [c]);
  // Every grading row projected: the whole list, each tool disclosed.
  useLayoutEffect(() => {
    for (const intent of coverage(p)) c.send(intent);
  }, [p]);
  const editable = p.target === "Layer" && !p.locked;
  const send = (intent: Intent) => {
    if (editable) c.send(intent);
  };
  const tools = grade(p);
  // The first Color Wheels is the section's own; any further one (each grades
  // the picture) follows as a tool of its own.
  const [wheels, ...moreWheels] = tools.filter(
    (g) => g.header.definition === WHEELS,
  );
  const curves = tools.filter((g) => g.header.definition === CURVES);
  const luts = tools.filter((g) => g.header.definition === LUT);
  const transforms = tools.filter((g) => g.header.definition === CST);
  const others = tools.filter(
    (g) => ![WHEELS, CURVES, LUT, CST].includes(g.header.definition ?? ""),
  );
  const rows = (list: (ParameterRow | undefined)[]) =>
    list.map(
      (row) =>
        row && <Control key={row.id} row={row} locked={p.locked} model={c} />,
    );
  // Balance and Tone as rows, then the four wheels (Lift, Gamma, Gain,
  // Offset) side by side as the panel's width allows.
  const wheelControls = (g: GradeEffect) => [
    <Plate key={`${g.header.id}:balance`} label="Balance and Tone" wide>
      {WHEEL_SECTIONS.slice(0, 2).map((s) => (
        <div key={`${g.header.id}:${s.title}`} role="group" label={s.title}>
          <Subheading title={s.title} />
          {rows(s.keys.map((k) => parameter(g, k)))}
        </div>
      ))}
    </Plate>,
    <Wheels key={`${g.header.id}:wheels`} g={g} locked={p.locked} />,
  ];
  const add = (definition: string, label: string) => (
    <SectionAction
      id={`${PANEL_ID}:add:${definition}`}
      label={label}
      short="Add"
      symbol="plus"
      disabled={!editable}
      onActivate={() => send({ type: "add", definition })}
    />
  );
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
        <>
          <Heading title={p.name} detail={p.typeLabel} />
          <scroll
            id={`${PANEL_ID}:content`}
            role="group"
            label={PANEL_TITLE}
            style={{ display: "flex", flexDirection: "column" }}
          >
            <Section
              title="Color Wheels"
              tool={wheels}
              send={send}
              editable={editable}
              action={
                <>
                  {wheels ? null : add(WHEELS, "Add Color Wheels")}
                  <SectionAction
                    id={`${PANEL_ID}:import-cdl`}
                    label="Import CDL…"
                    short="Import"
                    symbol="square.and.arrow.down"
                    disabled={!editable}
                    onActivate={() => command(IMPORT_CDL)}
                  />
                  {wheels && (
                    <SectionAction
                      id={`${PANEL_ID}:export-cdl`}
                      label="Export CDL…"
                      short="Export"
                      symbol="square.and.arrow.up"
                      disabled={p.target !== "Layer"}
                      onActivate={() => command(EXPORT_CDL)}
                    />
                  )}
                </>
              }
            >
              {wheels ? wheelControls(wheels) : null}
              {moreWheels.map((g) => (
                <Tool
                  key={g.header.id}
                  tool={g}
                  send={send}
                  editable={editable}
                >
                  {wheelControls(g)}
                </Tool>
              ))}
            </Section>
            <Section
              title="Curves"
              send={send}
              editable={editable}
              action={
                <SectionAction
                  id={`${PANEL_ID}:add:${CURVES}`}
                  label="Add Curve"
                  short="Add"
                  symbol="plus"
                  disabled={!editable}
                  onActivate={() => {
                    // A curve is made on the channel chosen, in one Undo step.
                    void c.platform
                      .contextMenu(
                        CURVE_CHANNELS.map((label, v) => ({
                          id: String(v),
                          label: `${label} Curve`,
                        })),
                      )
                      .then((choice) => {
                        if (choice !== null)
                          send({
                            type: "add",
                            definition: CURVES,
                            parameters: { channel: Number(choice) },
                          });
                      })
                      .catch((error) => c.platform.report(error));
                  }}
                />
              }
            >
              {curves.map((g) => (
                <Tool
                  key={g.header.id}
                  tool={g}
                  send={send}
                  editable={editable}
                >
                  {rows([parameter(g, "channel")])}
                  <CurveControl g={g} locked={p.locked} />
                </Tool>
              ))}
            </Section>
            <Section
              title="LUT"
              send={send}
              editable={editable}
              action={
                <SectionAction
                  id={`${PANEL_ID}:load-lut`}
                  label="Load LUT…"
                  symbol="folder"
                  disabled={!editable}
                  onActivate={() => command(LOAD_LUT)}
                />
              }
            >
              {luts.map((g) => (
                <Tool
                  key={g.header.id}
                  tool={g}
                  send={send}
                  editable={editable}
                >
                  {rows(g.parameters)}
                </Tool>
              ))}
            </Section>
            <Section
              title="Color Space Transform"
              send={send}
              editable={editable}
              action={add(CST, "Add Color Space Transform")}
            >
              {transforms.map((g) => (
                <Tool
                  key={g.header.id}
                  tool={g}
                  send={send}
                  editable={editable}
                >
                  {rows(g.parameters)}
                </Tool>
              ))}
            </Section>
            {others.length > 0 && (
              <Section title="More" send={send} editable={editable}>
                {others.map((g) => (
                  <Tool
                    key={g.header.id}
                    tool={g}
                    send={send}
                    editable={editable}
                  >
                    {rows(g.parameters)}
                  </Tool>
                ))}
              </Section>
            )}
          </scroll>
        </>
      )}
    </div>
  );
}

/** Lift, Gamma, Gain and Offset: each a drawn wheel over its master and
 * its red, green and blue (as Resolve shows them), wrapping two across in a
 * narrow panel and four across in a wide one. */
function Wheels({
  g,
  locked,
}: {
  key?: string;
  g: GradeEffect;
  locked: boolean;
}) {
  return (
    <div
      role="group"
      label="Wheels"
      style={{
        display: "flex",
        flexDirection: "row",
        flexWrap: "wrap",
        justifyContent: "center",
        columnGap: t("metrics.gap"),
        rowGap: t("metrics.gap"),
        paddingTop: t("metrics.gap"),
        paddingLeft: t("metrics.gap"),
        paddingRight: t("metrics.gap"),
      }}
    >
      {WHEELS_OF.map((w) => {
        const group = wheelGroup(g, w.key);
        const master = parameter(g, `${w.key}Master`);
        const rgb = (group?.value ?? [w.neutral, w.neutral, w.neutral]) as [
          number,
          number,
          number,
        ];
        const disabled = locked || !g.header.enabled || !group;
        return (
          // One instrument: its name, the wheel (its master on the bezel),
          // the master's field and the channels, on one plate.
          <div
            key={w.key}
            role="group"
            label={w.title}
            data-pm-variant="plate"
            style={{
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              gap: t("metrics.gap"),
              padding: t("metrics.gap"),
            }}
          >
            <text
              style={{
                color: t("colors.secondary"),
                fontSize: t("typography.label"),
                fontWeight: t("typography.semibold_weight"),
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
            {master && (
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
                style={{ width: t("metrics.grade_wheel") }}
              />
            )}
            <div
              role="group"
              label={`${w.title} red ${signed(rgb[0])}, green ${signed(rgb[1])}, blue ${signed(rgb[2])}`}
              style={{
                display: "flex",
                flexDirection: "row",
                justifyContent: "space-between",
                width: t("metrics.grade_wheel"),
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
      })}
    </div>
  );
}

/** A group of grading controls on the grading material (the host draws it:
 * the kit's one switch between the materials under review). */
function Plate({
  label,
  children,
  wide,
}: {
  key?: string;
  label: string;
  children: any;
  /** Spans the panel, inset from its edges (else it hugs its content). */
  wide?: boolean;
}) {
  return (
    <div
      role="group"
      label={label}
      data-pm-variant="plate"
      style={{
        display: "flex",
        flexDirection: "column",
        paddingTop: t("metrics.gap"),
        paddingBottom: t("metrics.gap"),
        paddingLeft: t("metrics.gap"),
        paddingRight: t("metrics.gap"),
        ...(wide
          ? {
              marginLeft: t("metrics.inset"),
              marginRight: t("metrics.inset"),
              marginTop: t("metrics.gap"),
            }
          : {}),
      }}
    >
      {children}
    </div>
  );
}

/** A channel value as a colourist reads it: two places, a true minus. */
function signed(v: number) {
  return v.toFixed(2).replace("-", "\u2212");
}

/** One curve's drawn editor, on the panel's inset. */
function CurveControl({ g, locked }: { g: GradeEffect; locked: boolean }) {
  const group = curveGroup(g);
  if (!group) return null;
  const channel = Number(parameter(g, "channel")?.parameter.value ?? 0);
  return (
    <div role="group" label="Curve" style={{ paddingTop: t("metrics.gap") }}>
      <UI.CurveEditor
        id={`${PANEL_ID}:curve:${g.header.id}`}
        label="Curve"
        binding={group.binding}
        value={group.value}
        channel={channel}
        disabled={locked || !g.header.enabled}
        onKeyDown={stop}
      />
    </div>
  );
}

function Heading({ title, detail }: { title: string; detail: string }) {
  return (
    <div
      role="heading"
      label={detail ? `${title} · ${detail}` : title}
      style={{
        display: "flex",
        flexDirection: "row",
        alignItems: "center",
        gap: t("metrics.gap"),
        height: t("metrics.row"),
        minHeight: t("metrics.row"),
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.inset"),
        color: t("colors.text"),
        // As Properties names its layer: the label rank, the kind after the
        // name in the muted ink (design language, Properties).
        fontSize: t("typography.label"),
        fontWeight: t("typography.semibold_weight"),
      }}
    >
      <text
        style={{
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
            fontWeight: t("typography.regular_weight"),
          }}
        >
          {`· ${detail}`}
        </text>
      )}
    </div>
  );
}

function Subheading({ title }: { title: string }) {
  return (
    <text
      role="heading"
      style={{
        paddingLeft: t("metrics.inset"),
        paddingTop: t("metrics.gap"),
        color: t("colors.secondary"),
        fontSize: t("typography.label"),
        fontWeight: t("typography.semibold_weight"),
      }}
    >
      {title}
    </text>
  );
}

/** A section; with `tool`, its header carries the tool's switch and Reset. */
function Section({
  title,
  tool,
  send,
  editable,
  action,
  children,
}: {
  title: string;
  tool?: GradeEffect;
  send: (intent: Intent) => void;
  editable: boolean;
  /** The section's own action, at the trailing edge of its heading. */
  action?: any;
  children: any;
}) {
  return (
    <div
      role="group"
      label={title}
      style={{ paddingBottom: t("metrics.inset") }}
    >
      <ToolHeader
        title={title}
        tool={tool}
        send={send}
        editable={editable}
        action={action}
      />
      {children}
    </div>
  );
}

function Tool({
  tool,
  send,
  editable,
  children,
}: {
  key?: string;
  tool: GradeEffect;
  send: (intent: Intent) => void;
  editable: boolean;
  children: any;
}) {
  const channel = parameter(tool, "channel");
  const label =
    channel?.parameter.options?.find((o) => o.v === channel.parameter.value)
      ?.label ?? tool.header.label;
  return (
    <div role="group" label={label}>
      <ToolHeader
        title={label}
        tool={tool}
        send={send}
        editable={editable}
        removable
      />
      <Plate label={label} wide>
        {children}
      </Plate>
    </div>
  );
}

function ToolHeader({
  title,
  tool,
  send,
  editable,
  removable,
  action,
}: {
  title: string;
  tool?: GradeEffect;
  send: (intent: Intent) => void;
  editable: boolean;
  removable?: boolean;
  action?: any;
}) {
  const id = tool?.header.id;
  return (
    <div
      role="heading"
      label={title}
      style={{
        display: "flex",
        flexDirection: "row",
        alignItems: "center",
        gap: t("metrics.gap"),
        height: t("metrics.row"),
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.inset"),
        color: t("colors.text"),
        // Section and tool headings: the label rank (design language,
        // Properties).
        fontSize: t("typography.label"),
        fontWeight: t("typography.semibold_weight"),
      }}
    >
      {id && (
        <UI.Checkbox
          id={`${PANEL_ID}:on:${id}`}
          preventDefault={{ click: true }}
          label={`Enable ${title}`}
          binding={`${PANEL_ID}:enable:${id}`}
          value={tool!.header.enabled}
          disabled={!editable}
          onKeyDown={stop}
          onClick={(e) => {
            stop(e);
            send({ type: "toggle", id });
          }}
        />
      )}
      <text style={{ flexGrow: 1 }}>{title}</text>
      {action}
      {id && (
        <UI.ToolbarItem
          id={`${PANEL_ID}:reset:${id}`}
          label={`Reset ${title}`}
          title="Reset"
          disabled={!editable}
          attr={{ symbol: "arrow.counterclockwise" }}
          onClick={(e) => {
            stop(e);
            send({ type: "reset", id });
          }}
        />
      )}
      {id && removable && (
        <UI.ToolbarItem
          id={`${PANEL_ID}:remove:${id}`}
          label={`Remove ${title}`}
          title="Remove"
          disabled={!editable}
          attr={{ symbol: "minus.circle" }}
          onClick={(e) => {
            stop(e);
            send({ type: "remove", id });
          }}
        />
      )}
    </div>
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
