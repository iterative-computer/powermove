import type { SurfaceEvent } from "powermove-v4";

// DESIGN-TODO(opus): neutral behavior fixture only; row composition/visuals need review.
// Inputs/requests below are synthetic test seams, not proposed Powermove host APIs.
export interface LayerRow {
  id: string;
  label: string;
  parent: string | null;
  group: boolean;
  collapsed: boolean;
  locked: boolean;
}
export function visibleRows(rows: LayerRow[]): LayerRow[] {
  const byId = new Map(rows.map((row) => [row.id, row]));
  return rows.filter((row) => {
    const visited = new Set([row.id]);
    let parent = row.parent;
    while (parent !== null) {
      if (visited.has(parent)) throw Error("Cyclic fixture hierarchy");
      visited.add(parent);
      const ancestor = byId.get(parent);
      if (!ancestor) throw Error("Missing fixture parent");
      if (ancestor.collapsed) return false;
      parent = ancestor.parent;
    }
    return true;
  });
}
/** Port of v1.2.0 selectLayerForPointer's stable range anchor and toggle behavior.
 * metaKey/ctrlKey are absent from SurfaceEvent; toggle is tested at this pure seam.
 */
export function selectRow(
  ids: string[],
  selected: string[],
  anchor: string | null,
  id: string,
  shift: boolean,
  toggle: boolean,
) {
  if (!ids.includes(id)) throw Error("Unknown row");
  if (shift) {
    if (!anchor || !ids.includes(anchor) || !selected.includes(anchor))
      anchor = selected.find((key) => ids.includes(key)) ?? id;
    const from = ids.indexOf(anchor),
      to = ids.indexOf(id);
    const range = ids.slice(Math.min(from, to), Math.max(from, to) + 1);
    return {
      anchor,
      selected: toggle ? [...new Set([...selected, ...range])] : range,
    };
  }
  return {
    anchor: id,
    selected: toggle
      ? selected.includes(id)
        ? selected.filter((key) => key !== id)
        : [...selected, id]
      : selected.includes(id)
        ? selected
        : [id],
  };
}
export function TimelineRows(props: {
  rows: LayerRow[];
  selected: string[];
  anchor: string | null;
  start: number;
  size: number;
  onSelect: (selection: { selected: string[]; anchor: string | null }) => void;
  onCollapse: (id: string, collapsed: boolean) => void;
}) {
  const rows = visibleRows(props.rows),
    start = Math.min(Math.max(0, props.start), rows.length),
    end = Math.min(start + props.size, rows.length);
  const select = (row: LayerRow, event: SurfaceEvent) =>
    props.onSelect(
      selectRow(
        rows.map((row) => row.id),
        props.selected,
        props.anchor,
        row.id,
        !!event.shiftKey,
        false,
      ),
    );
  return (
    <virtual-list
      id="timeline"
      count={rows.length}
      range={[start, end]}
      itemSize={1}
    >
      {rows.slice(start, end).map((row) => (
        <div
          key={row.id}
          id={`row:${row.id}`}
          role="row"
          state={{
            selected: props.selected.includes(row.id),
            locked: row.locked,
          }}
        >
          {row.group && (
            <button
              id={`disclosure:${row.id}`}
              label={row.label}
              state={{ expanded: !row.collapsed }}
              onClick={() => props.onCollapse(row.id, !row.collapsed)}
            />
          )}
          <button
            id={`name:${row.id}`}
            label={row.label}
            onClick={(event) => select(row, event)}
          />
        </div>
      ))}
    </virtual-list>
  );
}
