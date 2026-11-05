/** S025: evaluated display positions come from Rust; authored coordinates remain
 * in AX labels. The kit owns drawing, tokens, over-image palette and hit targets.
 * Native input queues receive IDs; this view never writes geometry. */
import { ViewerGizmo } from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
export type PathControlsPlan = Readonly<{
  pathId: string;
  vertices: readonly Readonly<{
    id: string;
    label: string;
    selected: boolean;
    hovered: boolean;
    position: { x: number; y: number };
    incoming: { x: number; y: number };
    outgoing: { x: number; y: number };
    showTangents: boolean;
  }>[];
}>;
export function PathControls(props: {
  plan: PathControlsPlan;
  input: (
    path: string,
    vertex: string,
    handle: "vertex" | "incoming" | "outgoing",
    event: SurfaceEvent,
  ) => void;
}) {
  const { plan } = props;
  return (
    <div>
      {plan.vertices.map((vertex) => (
        <div key={vertex.id}>
          <ViewerGizmo
            id={`path/${plan.pathId}/vertex/${vertex.id}`}
            kind="path-vertex"
            label={vertex.label}
            position={vertex.position}
            selected={vertex.selected}
            metadata={{ pmKit: { hovered: vertex.hovered } }}
            onPointerDown={(event) =>
              props.input(plan.pathId, vertex.id, "vertex", event)
            }
          />
          {vertex.showTangents &&
            (["incoming", "outgoing"] as const).map((handle) => (
              <ViewerGizmo
                key={handle}
                id={`path/${plan.pathId}/vertex/${vertex.id}/${handle}`}
                kind="tangent"
                label={vertex.label}
                aria-hidden={true}
                position={vertex[handle]}
                onPointerDown={(event) =>
                  props.input(plan.pathId, vertex.id, handle, event)
                }
              />
            ))}
        </div>
      ))}
    </div>
  );
}
