import { useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { ExportModel, QueueJob } from "./model";
const { token: t } = UI;
const row = {
  display: "flex",
  flexDirection: "row",
  alignItems: "center",
} as const;
const stack = { display: "flex", flexDirection: "column" } as const;
const body = { color: t("colors.text"), fontSize: t("typography.body") };
const sectionLabel = {
  color: t("colors.secondary"),
  fontSize: t("typography.minimum"),
  fontWeight: t("typography.semibold_weight"),
  fontVariantNumeric: "tabular-nums",
};
const caption = { color: t("colors.muted"), fontSize: t("typography.minimum") };
export const jobActions = (status: QueueJob["status"]) =>
  status === "Rendering" || status === "Pending"
    ? ["cancel"]
    : status === "Failed"
      ? ["retry", "remove"]
      : status === "Done"
        ? ["finder", "remove"]
        : ["remove"];
export function jobFormat(job: QueueJob) {
  if (job.format === "prores")
    return `ProRes ${({ "422proxy": "422 Proxy", "422lt": "422 LT", "422": "422", "422hq": "422 HQ", "4444": "4444", "4444xq": "4444 XQ" } as Record<string, string>)[job.profile ?? "4444"] ?? ""}`.trim();
  return (
    (
      {
        mp4: "MP4",
        webm: "WebM",
        png: "PNG sequence",
        still: "PNG",
        json: "Project",
      } as Record<string, string>
    )[job.format] ?? job.format
  );
}
export function jobTime(seconds?: number, now = new Date()) {
  if (!seconds) return "";
  const date = new Date(seconds * 1000);
  const day = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const yesterday = new Date(day);
  yesterday.setDate(day.getDate() - 1);
  const prefix =
    date >= day
      ? "Today"
      : date >= yesterday
        ? "Yesterday"
        : date.toLocaleDateString(undefined, {
            month: "short",
            day: "numeric",
          });
  return `${prefix} ${date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false })}`;
}
const icons = {
  cancel: ["xmark", "Cancel"],
  retry: ["arrow.clockwise", "Retry"],
  finder: ["magnifyingglass", "Show in Finder"],
  remove: ["trash", "Remove"],
} as const;
function JobRow({
  job,
  model,
  watch,
}: {
  key?: number;
  job: QueueJob;
  model: ExportModel;
  watch: (p: Promise<unknown>) => void;
}) {
  const [hover, setHover] = useState(false);
  const [focus, setFocus] = useState(false);
  const rendering = job.status === "Rendering";
  const symbol = {
    Pending: "clock",
    Rendering: "",
    Done: "checkmark.circle.fill",
    Failed: "exclamationmark.triangle.fill",
    Cancelled: "xmark.circle",
  }[job.status];
  const color =
    job.status === "Done"
      ? "colors.success"
      : job.status === "Failed"
        ? "colors.warning"
        : "colors.muted";
  const filename = job.path.split(/[\\/]/).filter(Boolean).at(-1) ?? job.path;
  return (
    <div
      id={`queue.job.${job.id}`}
      role="group"
      label={`${job.name}, ${job.status}`}
      title={job.error || job.path}
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
      onFocus={() => setFocus(true)}
      onBlur={(event) =>
        setFocus(
          jobActions(job.status).some(
            (action) => event.value === `queue.${action}.${job.id}`,
          ),
        )
      }
      style={{
        ...row,
        height: t("metrics.export_job_row"),
        minHeight: t("metrics.export_job_row"),
        gap: t("space.md"),
      }}
    >
      <div
        style={{
          ...row,
          justifyContent: "center",
          width: t("metrics.hit"),
          minWidth: t("metrics.hit"),
          flexShrink: 0,
        }}
      >
        {rendering ? (
          <UI.ProgressRow
            id={`queue.ring.${job.id}`}
            label={`${Math.round(job.progress * 100)} percent rendered`}
            value={job.progress}
            children={[]}
            metadata={{ pmKit: { queueProgress: "ring" } }}
            style={{
              width: t("metrics.symbol_panel"),
              height: t("metrics.symbol_panel"),
            }}
          />
        ) : (
          <div
            attr={{ symbol }}
            label={job.status}
            style={{
              color: t(color),
              symbolSize: t("metrics.symbol_panel"),
              width: t("metrics.symbol_panel"),
              height: t("metrics.symbol_panel"),
            }}
          />
        )}
      </div>
      <div style={{ ...stack, flexGrow: 1, flexShrink: 1, overflow: "hidden" }}>
        <text
          style={{
            ...body,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {job.name}
        </text>
        <text
          title={job.path}
          style={{
            ...caption,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >{`${jobFormat(job)} · ${filename}`}</text>
      </div>
      {rendering ? (
        <div style={{ ...row, gap: t("space.sm"), flexShrink: 0 }}>
          <UI.ProgressRow
            id={`queue.bar.${job.id}`}
            label={`${job.name} progress`}
            value={job.progress}
            children={[]}
            metadata={{ pmKit: { queueProgress: "bar" } }}
            style={{
              width: t("metrics.export_job_progress_width"),
              height: t("metrics.export_job_progress_height"),
            }}
          />
          <text
            style={{ ...caption, fontVariantNumeric: "tabular-nums" }}
          >{`${Math.round(job.progress * 100)} %`}</text>
        </div>
      ) : (
        <text
          id={`queue.time.${job.id}`}
          style={{
            ...caption,
            fontVariantNumeric: "tabular-nums",
            whiteSpace: "nowrap",
          }}
        >
          {jobTime(job.finishedAt)}
        </text>
      )}
      <div style={{ ...row, flexShrink: 0, opacity: hover || focus ? 1 : 0 }}>
        {jobActions(job.status).map((action) => {
          const [symbol, label] = icons[action as keyof typeof icons];
          return (
            <UI.ToolbarItem
              key={action}
              id={`queue.${action}.${job.id}`}
              label={`${label} ${job.name}`}
              title={label}
              attr={{ symbol }}
              metadata={{ pmKit: { recipe: "icon_button" } }}
              style={{
                width: t("metrics.hit"),
                height: t("metrics.hit"),
                symbolSize: t("metrics.symbol_inline"),
              }}
              onClick={() =>
                watch(
                  action === "finder"
                    ? model.command("export.queue.finder", { id: job.id })
                    : model.queueAction(job.id, action),
                )
              }
            />
          );
        })}
      </div>
    </div>
  );
}
export function RenderQueue({
  model,
  watch,
}: {
  model: ExportModel;
  watch: (p: Promise<unknown>) => void;
}) {
  const jobs = model.state.queue ?? [];
  const pending = jobs.filter(
    (j) => j.status === "Pending" || j.status === "Rendering",
  );
  const history = jobs
    .filter((j) => j.status !== "Pending" && j.status !== "Rendering")
    .sort(
      (a, b) => (b.finishedAt ?? b.id) - (a.finishedAt ?? a.id) || b.id - a.id,
    );
  return (
    <div id="export.queue" style={{ ...stack, gap: t("space.lg") }}>
      <div role="group" label="Queue" style={stack}>
        <div
          style={{
            ...row,
            justifyContent: "space-between",
            height: t("metrics.hit"),
            marginBottom: t("space.xs"),
          }}
        >
          <text style={sectionLabel}>{`Queue · ${pending.length}`}</text>
          <UI.PushButton
            id="export.queue.render"
            label="Render Pending"
            title="Render pending jobs"
            disabled={
              !pending.some((j) => j.status === "Pending") ||
              pending.some((j) => j.status === "Rendering")
            }
            style={{
              fontSize: t("typography.minimum"),
              height: t("metrics.hit"),
            }}
            onClick={() => watch(model.command("export.queue.render"))}
          />
        </div>
        {pending.length ? (
          pending.map((job) => (
            <JobRow key={job.id} job={job} model={model} watch={watch} />
          ))
        ) : (
          <UI.EmptyState
            label="Queue is empty"
            explanation="Add a composition to render it later."
            symbol="clock"
          />
        )}
      </div>
      <div role="group" label="History" style={stack}>
        <div
          style={{
            ...row,
            justifyContent: "space-between",
            height: t("metrics.hit"),
            marginBottom: t("space.xs"),
          }}
        >
          <text style={sectionLabel}>History</text>
          <UI.ToolbarItem
            id="export.queue.clear"
            label="Clear"
            title="Clear finished jobs"
            disabled={!history.length}
            style={{ fontSize: t("typography.minimum") }}
            onClick={() => watch(model.queueAction(0, "clear"))}
          >
            Clear
          </UI.ToolbarItem>
        </div>
        {history.length ? (
          history.map((job) => (
            <JobRow key={job.id} job={job} model={model} watch={watch} />
          ))
        ) : (
          <UI.EmptyState
            label="No export history"
            explanation="Finished exports appear here."
            symbol="clock"
          />
        )}
      </div>
    </div>
  );
}
