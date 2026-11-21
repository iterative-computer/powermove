import { expect, test } from "bun:test";
import { createRoot } from "../../src/index";
import type { Row } from "./catalogue";
import { bindings, fixture } from "./fixtures";
import { SettingsPanel } from "./index";

test("Other Agent Apps identifies every icon in its Settings row", async () => {
  const apps = [
    ["claude-code", "Claude Code"],
    ["codex", "Codex"],
    ["claude-desktop", "Claude"],
    ["gemini-cli", "Gemini CLI"],
    ["opencode", "OpenCode"],
    ["other", "Any Other Agent"],
  ] as const;
  const rows: Row[] = apps.map(([id, label]) => ({
    id: `agents.${id}`,
    section: "agents",
    group: "Other Agent Apps",
    label,
    detail: `Set up ${label}.`,
    control: "status",
    value: id === "claude-desktop" ? "Added" : "",
  }));
  rows.push({
    id: "account.chatgpt",
    section: "agents",
    group: "Subscriptions",
    label: "ChatGPT",
    control: "status",
    value: "Signed in",
  });
  const { model } = fixture({ rows });
  model.show("agents");
  const root = createRoot(async (commit) => ({
    accepted: true,
    revision: commit.baseRevision + 1,
  }));
  await root.render(<SettingsPanel model={model} bindings={bindings} />);
  await root.flush();

  const nodes = root.inspect().nodes;
  for (const [id] of apps) {
    const row = nodes.find((node) => node.props.id === `settings.row.agents.${id}`);
    expect(row?.props.metadata).toEqual({ settingsLayout: "row", agentApp: id });
  }
  const subscription = nodes.find((node) => node.props.id === "settings.row.account.chatgpt");
  expect(subscription?.props.metadata).toEqual({ settingsLayout: "row" });
  const styleForText = (text: string) => {
    const content = nodes.find((node) => node.text === text);
    expect(content).toBeDefined();
    const label = nodes.find((node) => node.children.includes(content!.id));
    expect(label).toBeDefined();
    return label?.props.style;
  };
  expect(styleForText("Added")).toEqual({ whiteSpace: "nowrap" });
  expect(styleForText("Set up Claude.")).toBeUndefined();
  expect(styleForText("Signed in")).toBeUndefined();
  root.dispose();
  model.dispose();
});
