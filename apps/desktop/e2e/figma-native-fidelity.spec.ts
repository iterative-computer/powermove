import { test, expect } from "./helpers/app";
import {
  validateFigmaDoc,
  type FigmaNode,
  type Paint,
} from "../src/shared/figma/doc";

const solid = (r = 0.95, g = 0.3, b = 0.12, a = 1): Paint => ({
  type: "SOLID",
  color: { r, g, b, a },
});
const node = (id: string, patch: Partial<FigmaNode> = {}): FigmaNode => ({
  id,
  fileKey: "synthetic-native-reference",
  name: id,
  type: "RECTANGLE",
  w: 240,
  h: 150,
  absoluteTransform: [1, 0, 0, 1, 0, 0],
  fills: [solid()],
  strokes: [],
  effects: [],
  children: [],
  ...patch,
});
async function send(
  session: any,
  nodes: FigmaNode[],
  w = 300,
  h = 210,
  images: Record<string, string> = {},
) {
  await session.openEditor();
  await session.page.evaluate(
    ({ w, h }) => {
      const P = (window as any).PM;
      P.replaceProject(P.mkProject({ w, h, dur: 8, bg: "#272727" }));
      P.setTime(0);
    },
    { w, h },
  );
  const doc = validateFigmaDoc({
    format: "powermove-figma",
    version: 1,
    source: "plugin",
    nodes,
    images,
  });
  const result = await session.app.evaluate(
    async ({ ipcMain, BrowserWindow }: any, doc: any) =>
      new Promise((resolve, reject) => {
        const id = "native-reference";
        const listener = (_: any, v: any) => {
          if (v?.id !== id) return;
          clearTimeout(timer);
          ipcMain.off("figma:result", listener);
          resolve(v.result);
        };
        const timer = setTimeout(() => {
          ipcMain.off("figma:result", listener);
          reject(Error("Import timed out"));
        }, 15000);
        ipcMain.on("figma:result", listener);
        BrowserWindow.getAllWindows()[0].webContents.send("figma:receive", {
          id,
          doc,
        });
      }),
    doc,
  );
  expect(result.error).toBeUndefined();
  expect(result.flattened).toBe(0);
}

test("equilateral arrow heads and their shafts match actual Figma strokeGeometry", async ({
  session,
}) => {
  await send(
    session,
    [
      node("arrow", {
        type: "VECTOR",
        h: 80,
        fills: [],
        strokes: [solid()],
        strokeWeight: 8,
        strokeAlign: "CENTER",
        paths: [
          {
            data: "M0 80 C0 80 240 0 240 0",
            windingRule: "NONZERO",
            startCap: "NONE",
            endCap: "ARROW_EQUILATERAL",
          },
        ],
      }),
    ],
    300,
    140,
  );
  const result = await session.page.evaluate(async () => {
    const P = (window as any).PM,
      actual = P.renderFrameTo(0, 300, 140),
      ref = document.createElement("canvas");
    ref.width = 300;
    ref.height = 140;
    const c = ref.getContext("2d")!;
    c.fillStyle = "#272727";
    c.fillRect(0, 0, 300, 140);
    // Exported by real Figma for an eight-pixel shaft from (0,80) to (240,0).
    const path =
      "M0 80 L1.26491 83.7947 L207.112 15.1789 L205.847 11.3842 L204.582 7.58947 L-1.26491 76.2053 L0 80 Z M240 0 L194.75 -9.25979 L209.356 34.558 L240 0 Z";
    c.translate(30, 30);
    c.fillStyle = "#f24d1f";
    c.fill(new Path2D(path));
    const a = actual.getContext("2d").getImageData(0, 0, 300, 140).data,
      r = c.getImageData(0, 0, 300, 140).data;
    let sum = 0,
      coverage = 0;
    for (let i = 0; i < a.length; i += 4) {
      for (let k = 0; k < 3; k++) sum += Math.abs(a[i + k] - r[i + k]);
      if (r[i] > 150 && r[i + 1] < 110) coverage++;
    }
    return { mean: sum / (300 * 140 * 3), coverage };
  });
  expect(result.coverage).toBeGreaterThan(2200);
  expect(result.mean).toBeLessThan(0.35);
});

test("transparent diamond stops match native Figma PNG samples", async ({
  session,
}) => {
  await send(session, [
    node("diamond", {
      fills: [
        {
          type: "GRADIENT_DIAMOND",
          gradientTransform: [1, 0.1, 0.15, 1, 0, -0.05],
          gradientStops: [
            { position: 0, color: { r: 1, g: 0.15, b: 0, a: 1 } },
            { position: 0.3, color: { r: 1, g: 0.85, b: 0.1, a: 0.35 } },
            { position: 0.65, color: { r: 0.1, g: 0.8, b: 0.45, a: 1 } },
            { position: 1, color: { r: 0.1, g: 0.25, b: 1, a: 1 } },
          ],
        },
      ],
    }),
  ]);
  const samples = await session.page.evaluate(() => {
    const c = (window as any).PM.renderFrameTo(0, 300, 210).getContext("2d");
    return [
      [120, 75],
      [90, 60],
      [60, 75],
      [180, 50],
      [120, 40],
    ].map(([x, y]) =>
      Array.from(c.getImageData(x + 30, y + 30, 1, 1).data).slice(0, 3),
    );
  });
  const native = [
    [180, 101, 23],
    [120, 115, 38],
    [121, 128, 43],
    [26, 111, 208],
    [84, 175, 78],
  ];
  for (let i = 0; i < native.length; i++)
    for (let k = 0; k < 3; k++)
      expect(
        Math.abs(Number(samples[i]![k]) - native[i]![k]!),
      ).toBeLessThanOrEqual(2);
});

test("magnified Figma image fills preserve the native texel boundary", async ({
  session,
}) => {
  await session.openEditor();
  const png = await session.page.evaluate(() => {
    const c = document.createElement("canvas");
    c.width = 96;
    c.height = 64;
    const x = c.getContext("2d")!;
    for (let y = 0; y < 64; y += 16)
      for (let i = 0; i < 96; i += 16) {
        x.fillStyle = (i / 16 + y / 16) % 2 ? "#39a3ff" : "#ffd75b";
        x.fillRect(i, y, 16, 16);
      }
    return c.toDataURL().split(",")[1]!;
  });
  await send(
    session,
    [
      node("image", {
        fills: [{ type: "IMAGE", imageHash: "checker", scaleMode: "FILL" }],
      }),
    ],
    300,
    210,
    { checker: png },
  );
  const samples = await session.page.evaluate(() => {
    const c = (window as any).PM.renderFrameTo(0, 300, 210).getContext("2d");
    return [38, 39, 40, 41].map((x) =>
      Array.from(c.getImageData(x + 30, 40, 1, 1).data),
    );
  });
  expect(samples).toEqual([
    [255, 215, 91, 255],
    [255, 215, 91, 255],
    [57, 163, 255, 255],
    [57, 163, 255, 255],
  ]);
});

test("background blur clamps to its owner and matches native Figma stripe samples", async ({
  session,
}) => {
  const children = Array.from({ length: 8 }, (_, i) =>
    node("stripe" + i, {
      w: 30,
      h: 150,
      absoluteTransform: [1, 0, 0, 1, i * 30, 0],
      fills: [i % 2 ? solid() : solid(0.1, 0.5, 1)],
    }),
  );
  children.push(
    node("blur", {
      w: 180,
      h: 110,
      absoluteTransform: [1, 0, 0, 1, 30, 20],
      radii: [14, 14, 14, 14],
      fills: [solid(1, 1, 1, 0.22)],
      effects: [
        {
          type: "BACKGROUND_BLUR",
          radius: 16,
          visible: true,
          blurType: "NORMAL",
        },
      ],
    }),
  );
  await send(session, [node("group", { type: "GROUP", fills: [], children })]);
  const result = await session.page.evaluate(() => {
    const P = (window as any).PM,
      c = P.renderFrameTo(0, 300, 210).getContext("2d");
    return {
      versions: P.proj.layers.flatMap((l: any) =>
        l.fx
          .filter((f: any) => f.type === "figmaBackdropBlur")
          .map((f: any) => f.version),
      ),
      samples: [
        [31, 35],
        [33, 30],
        [60, 75],
        [105, 75],
      ].map(([x, y]) =>
        Array.from(c.getImageData(x + 30, y + 30, 1, 1).data).slice(0, 3),
      ),
      errors: Array.from(P.GL.errors.entries()),
    };
  });
  const native = [
    [243, 116, 80],
    [244, 117, 81],
    [156, 137, 173],
    [244, 116, 81],
  ];
  expect(result.versions).toEqual([2]);
  expect(result.errors).toEqual([]);
  for (let i = 0; i < native.length; i++)
    for (let k = 0; k < 3; k++)
      expect(
        Math.abs(Number(result.samples[i]![k]) - native[i]![k]!),
      ).toBeLessThanOrEqual(6);
});

test("uniform native rectangle strokes remain editable as Outside and undo once", async ({
  session,
}) => {
  await send(session, [
    node("uniform stroke", {
      strokes: [solid(0.1, 0.5, 1)],
      strokeWeight: 10,
      strokeWeights: [10, 10, 10, 10],
      strokeAlign: "INSIDE",
      dashes: [18, 10],
      cap: "ROUND",
    }),
  ]);
  const { page } = session;
  await page.evaluate(() => {
    const P = (window as any).PM;
    P.selectLayers([
      P.proj.layers.find((layer: any) => layer.name === "uniform stroke").id,
    ]);
    P.hist.clear();
  });
  const render = () =>
    page.evaluate(() =>
      (window as any).PM.renderFrameTo(0, 300, 210).toDataURL(),
    );
  const before = await render();
  const outside = page
    .locator("#panel-inspector")
    .getByRole("button", { name: "Outside", exact: true });
  await expect(outside).toBeEnabled();
  await outside.click();
  await expect.poll(render).not.toBe(before);
  expect(
    await page.evaluate(
      () =>
        (window as any).PM.proj.layers.find(
          (layer: any) => layer.name === "uniform stroke",
        ).d.strokeStyle.align.v,
    ),
  ).toBe("outside");
  await page.keyboard.press("Meta+z");
  await expect.poll(render).toBe(before);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
