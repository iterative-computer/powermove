import { useState } from "powermove-v4/hooks";
// DESIGN-TODO(opus): neutral behavior fixture; category/search/list composition
// and focus interactions await design review. Labels come from synthetic data.
export interface EffectItem {
  id: string;
  label: string;
  group: string;
}
export function recentAfterApply(previous: string[], id: string): string[] {
  return [id, ...previous.filter((other) => other !== id)].slice(0, 8);
}
export function browseEffects(
  items: EffectItem[],
  query: string,
  category: string,
  recent: string[],
) {
  const categories = [...new Set(items.map((item) => item.group))];
  const recentItems = recent
    .map((id) => items.find((item) => item.id === id))
    .filter((item): item is EffectItem => !!item);
  const rails = [
    "all",
    ...(recentItems.length ? ["recent"] : []),
    ...categories,
  ];
  const active = rails.includes(category) ? category : "all",
    needle = query.trim().toLowerCase();
  const shown = needle
    ? items.filter(
        (item) =>
          item.label.toLowerCase().includes(needle) ||
          item.id.toLowerCase().includes(needle),
      )
    : active === "recent"
      ? recentItems
      : active === "all"
        ? items
        : items.filter((item) => item.group === active);
  return {
    rails,
    active,
    shown,
    sections:
      needle || active !== "all"
        ? [{ group: null, items: shown }]
        : categories.map((group) => ({
            group,
            items: shown.filter((item) => item.group === group),
          })),
  };
}
/** Requests are test-only. No PM globals, private services or direct document writes. */
export function EffectsBrowser(props: {
  items: EffectItem[];
  category: string;
  recent: string[];
  selectedLayer: string | null;
  onPickCategory: (id: string) => void;
  onApply: (layer: string, effect: string) => void;
  onMissingSelection: () => void;
}) {
  const [query, setQuery] = useState("");
  const state = browseEffects(props.items, query, props.category, props.recent);
  return (
    <div id="effects">
      <input
        id="effects:query"
        label="Search effects"
        value={query}
        onInput={(event) => setQuery(String(event.value ?? ""))}
      />
      <div>
        {state.rails.map((id) => (
          <button
            key={id}
            id={`category:${id}`}
            label={id}
            onClick={() => {
              setQuery("");
              props.onPickCategory(id);
            }}
          />
        ))}
      </div>
      <scroll>
        {state.sections.map((section) => (
          <div key={section.group ?? "flat"}>
            {section.items.map((item) => (
              <button
                key={item.id}
                id={`effect:${item.id}`}
                label={item.label}
                onClick={() => {
                  if (!props.selectedLayer) props.onMissingSelection();
                  else props.onApply(props.selectedLayer, item.id);
                }}
              />
            ))}
          </div>
        ))}
      </scroll>
    </div>
  );
}
