import { useState } from "powermove/hooks";

type Hot = { data: Record<string, any>; dispose(save: (data: Record<string, any>) => void): void };
let hot: Hot;
let latest = 0;

/** Runs once per version. What `dispose` saves is the next version's `hot.data`. */
export function activate(api: { hot: Hot }) {
  hot = api.hot;
  latest = hot.data.count ?? 0;
  hot.dispose((data) => {
    data.count = latest;
  });
}

/** A panel: Window › Hello. Its tree is native; nothing here is DOM. The count
 * survives a republish through `hot`. */
export function view() {
  const [count, setCount] = useState(latest);
  latest = count;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: 12 }}>
      <text>{`Clicked ${count} times`}</text>
      <button onClick={() => setCount(count + 1)}>Click</button>
    </div>
  );
}
