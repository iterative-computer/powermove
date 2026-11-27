import { NumField, Row, Section } from "powermove-v4/ui";
// DESIGN-TODO(opus): neutral semantic composition; no picker/layout/visual approval.
// The immutable evaluated input and request callback are test seams only. Real
// selection/time/query subscriptions and binding registration are absent publicly.
export interface PropertyRow {
  id: string;
  label: string;
  values: number[];
  binding: string;
  editable: boolean;
  animated: boolean;
  keyAtPlayhead: boolean;
  generic: boolean;
  min?: number;
  max?: number;
  step?: number;
  unit?: string;
  precision?: number;
}
export function InspectorGroup(props: {
  title: string;
  time: number;
  properties: PropertyRow[];
  onKey: (id: string, time: number) => void;
  onSelectChannel: (id: string) => void;
}) {
  return (
    <Section title={props.title}>
      {props.properties.map((property) => (
        <Row key={property.id} label={property.label}>
          <NumField
            id={`property:${property.id}`}
            value={property.values[0] ?? 0}
            binding={property.binding}
            mixed={property.values.some(
              (value) => value !== property.values[0],
            )}
            disabled={!property.editable}
            min={property.min}
            max={property.max}
            step={property.step}
            unit={property.unit}
            precision={property.precision}
            state={{
              animated: property.animated,
              keyAtPlayhead: property.keyAtPlayhead,
            }}
            onPointerDown={() => {
              if (!property.generic) props.onSelectChannel(property.id);
            }}
          />
          <button
            id={`key:${property.id}`}
            label={property.label}
            disabled={!property.editable}
            state={{ keyAtPlayhead: property.keyAtPlayhead }}
            onClick={() => {
              if (property.editable) props.onKey(property.id, props.time);
            }}
          />
        </Row>
      ))}
    </Section>
  );
}
