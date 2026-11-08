import { useState } from "powermove/hooks";

/** A panel: Window › Hello. Its tree is native; nothing here is DOM. */
export function view() {
  const [count, setCount] = useState(0);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: 12 }}>
      <text>{`Clicked ${count} times`}</text>
      <button onClick={() => setCount(count + 1)}>Click</button>
    </div>
  );
}
