import { expect, test } from "bun:test";
import type { HostClient } from "../../../src/host-api";
import { ViewerService } from "../service";
import { fixture } from "./helpers";

test("viewer publications retain the nested path and drive the observed composition ticket", async () => {
  let publish!: (state: unknown) => void;
  const commands: unknown[] = [];
  const host = {
    identity: { project: "p" },
    subscribe: async (_id: string, _query: string, next: typeof publish) => {
      publish = next;
      return async () => {};
    },
    command: async (...args: unknown[]) => {
      commands.push(args);
    },
  } as unknown as HostClient;
  const service = new ViewerService(host);
  await service.start("navigation", () => {});
  const state = fixture();
  state.model!.compositions = [
    { id: "main", name: "Main", parents: [] },
    { id: "logo", name: "Logo", parents: ["main"] },
    { id: "spinner", name: "Spinner", parents: ["logo"] },
  ];
  state.model!.composition = "spinner";
  publish(state);
  expect(service.observed!.model!.trail).toEqual({
    ids: ["main", "logo", "spinner"],
    index: 2,
  });
  state.model!.composition = "main";
  publish(state);
  expect(service.observed!.model!.trail).toEqual({
    ids: ["main", "logo", "spinner"],
    index: 0,
  });
  await service.drive("viewer.composition.open", "logo");
  expect(commands).toEqual([
    [
      "viewer.chrome.drive",
      { ticket: "1", element: "viewer.composition.open", argument: "logo" },
    ],
  ]);
  await service.stop();
});
