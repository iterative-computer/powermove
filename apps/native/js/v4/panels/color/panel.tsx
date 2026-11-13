import { useLayoutEffect, useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
import { ColorModel, type Intent, type ParameterRow } from "./model";
import {
  CURVES,
  LOAD_LUT,
  LUT,
  PANEL_ID,
  PANEL_TITLE,
  WHEELS,
  WHEEL_SECTIONS,
  coverage,
  curveRows,
  grade,
  parameter,
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
  const wheels = tools.find((g) => g.header.definition === WHEELS);
  const curves = tools.filter((g) => g.header.definition === CURVES);
  const luts = tools.filter((g) => g.header.definition === LUT);
  const others = tools.filter(
    (g) => ![WHEELS, CURVES, LUT].includes(g.header.definition ?? ""),
  );
  const rows = (list: (ParameterRow | undefined)[]) =>
    list.map(
      (row) =>
        row && <Control key={row.id} row={row} locked={p.locked} model={c} />,
    );
  const add = (definition: string, label: string) => (
    <UI.PushButton
      id={`${PANEL_ID}:add:${definition}`}
      label={label}
      disabled={!editable}
      onClick={(e) => {
        stop(e);
        send({ type: "add", definition });
      }}
      style={{ height: t("metrics.control"), marginTop: t("metrics.inset") }}
    >
      {label}
    </UI.PushButton>
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
            >
              {wheels
                ? WHEEL_SECTIONS.map((s) => (
                    <div key={s.title} role="group" label={s.title}>
                      <Subheading title={s.title} />
                      {rows(s.keys.map((k) => parameter(wheels, k)))}
                    </div>
                  ))
                : add(WHEELS, "Add Color Wheels")}
            </Section>
            <Section title="Curves" send={send} editable={editable}>
              {curves.map((g) => (
                <Tool
                  key={g.header.id}
                  tool={g}
                  send={send}
                  editable={editable}
                >
                  {rows(curveRows(g))}
                </Tool>
              ))}
              {add(CURVES, "Add Curve")}
            </Section>
            <Section title="LUT" send={send} editable={editable}>
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
              <UI.PushButton
                id={`${PANEL_ID}:load-lut`}
                label="Load LUT…"
                disabled={!editable}
                onClick={(e) => {
                  stop(e);
                  command(LOAD_LUT);
                }}
                style={{
                  height: t("metrics.control"),
                  marginTop: t("metrics.inset"),
                }}
              >
                Load LUT…
              </UI.PushButton>
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
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.inset"),
        color: t("colors.text"),
        fontSize: t("typography.title"),
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
        color: t("colors.muted"),
        fontSize: t("typography.caption"),
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
  children,
}: {
  title: string;
  tool?: GradeEffect;
  send: (intent: Intent) => void;
  editable: boolean;
  children: any;
}) {
  return (
    <div
      role="group"
      label={title}
      style={{ paddingBottom: t("metrics.inset") }}
    >
      <ToolHeader title={title} tool={tool} send={send} editable={editable} />
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
      {children}
    </div>
  );
}

function ToolHeader({
  title,
  tool,
  send,
  editable,
  removable,
}: {
  title: string;
  tool?: GradeEffect;
  send: (intent: Intent) => void;
  editable: boolean;
  removable?: boolean;
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
        fontSize: t("typography.body"),
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
              symbolSize: t("metrics.keyframe"),
            }}
          />
        }
      >
        {field}
      </UI.InspectorRow>
    </div>
  );
}
