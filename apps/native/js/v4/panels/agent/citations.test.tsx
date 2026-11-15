import { test, expect } from "bun:test";
import { createRoot, type Commit } from "../../src";
import { AgentView } from "./view";
import { citationLabel, quoteSelection, isCitation } from "./citations";
import { withRows } from "./fixtures";

test("UTF-16 quote offsets and Unicode truncation", () => {
  expect(quoteSelection("m", "A😀 exact", 1, 9)).toEqual({
    messageId: "m",
    text: "😀 exact",
    start: 1,
    end: 9,
    comment: "",
  });
  expect(quoteSelection("m", "abc", 3, 1)).toBeNull();
  expect(citationLabel("😀".repeat(33))).toBe("😀".repeat(32) + "…");
  expect(citationLabel("short")).toBe("short");
  expect(quoteSelection("m", "😀", 0, 1)).toBeNull();
  expect(
    isCitation({ messageId: "m", text: "😀", start: 0, end: 1, comment: "" }),
  ).toBe(false);
});
test("select, cite, comment, jump, remove and sent history", async () => {
  const commits: Commit[] = [];
  const p = withRows([
    {
      id: "history:m",
      commandId: "m",
      chosen: {},
      open: false,
      value: {
        role: "assistant",
        text: "If you've already picked a mode for your project, keep it.",
      },
    },
  ]);
  const root = createRoot(async (c) => {
    commits.push(structuredClone(c));
    return { accepted: true, revision: c.baseRevision + 1 };
  });
  const render = () =>
    root.render(
      <AgentView
        publication={p}
        send={(a) => {
          if (a.type === "citations") p.snapshot.draft.citations = a.citations;
        }}
      />,
    );
  const node = (label: string) => {
    const n = root
      .inspect()
      .nodes.find((n) => n.props.label === label && n.props.role !== "group");
    if (!n) throw Error(label);
    return n;
  };
  const capture = async (name: string) => {
    if (process.env.PM_CITE_STATES)
      await Bun.write(
        `${process.env.PM_CITE_STATES}/${name}.json`,
        JSON.stringify(commits),
      );
  };
  await render();
  await capture("after-idle");
  await root.drive(node("Transcript text").id, "select", {
    selection: [0, 40],
  });
  await capture("cite-pill");
  await root.drive(node("Cite selection").id, "click");
  await render();
  expect(p.snapshot.draft.citations?.[0]?.end).toBe(40);
  expect(p.snapshot.draft.citations?.[0]?.text).toBe(
    String(p.snapshot.rows.items[0]!.value.text).slice(0, 40),
  );
  await capture("chips");
  await root.drive(node("Edit comment").id, "click");
  await capture("comment");
  await root.drive(node("Add a comment about this quote").id, "input", {
    value: "Explain this choice",
  });
  await root.drive(node("Add a comment about this quote").id, "keydown", {
    key: "Enter",
  });
  await render();
  expect(p.snapshot.draft.citations?.[0]?.comment).toBe("Explain this choice");
  await root.drive(
    node(`Quoted: ${p.snapshot.draft.citations![0]!.text}`).id,
    "click",
  );
  expect((node("Transcript text").props.metadata as any).revealQuote).toEqual([
    0, 40, 1,
  ]);
  await root.drive(node("Edit comment").id, "click");
  await root.drive(node("Add a comment about this quote").id, "input", {
    value: "Discard this",
  });
  await root.drive(node("Add a comment about this quote").id, "keydown", {
    key: "Escape",
  });
  expect(p.snapshot.draft.citations?.[0]?.comment).toBe("Explain this choice");
  const quotes = structuredClone(p.snapshot.draft.citations!);
  await root.drive(node("Remove quote").id, "click");
  expect(p.snapshot.draft.citations).toEqual([]);
  await render();
  await root.drive(node("Transcript text").id, "select", { selection: [1, 6] });
  await root.drive(node("Transcript text").id, "keydown", {
    key: ".",
    metaKey: true,
    shiftKey: true,
  });
  expect(p.snapshot.draft.citations?.[0]?.text).toBe("f you");
  await render();
  await root.drive(node("Remove quote").id, "click");
  p.snapshot.rows.items.push({
    id: "history:u",
    commandId: "u",
    chosen: {},
    open: false,
    value: {
      role: "user",
      text: "Help me understand",
      citations: quotes as any,
    },
  });
  p.snapshot.rows.count++;
  await render();
  expect(
    root.inspect().nodes.some((n) => n.text === "Explain this choice"),
  ).toBe(true);
  await capture("sent");
  root.dispose();
});
