import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { build } from "esbuild";
import { test, expect } from "./helpers/app";
import { validateFigmaDoc } from "../src/shared/figma/doc";
import { allFeatures } from "../src/shared/figma/__fixtures__/all-features";

test("the complete fixture grid matches independent references, including invisible gray HSL blends", async ({
  session,
}, info) => {
  test.setTimeout(90000);
  await session.openEditor();
  const { app, page } = session,
    w = 930,
    h = 680,
    doc = allFeatures();
  await app.evaluate(({ BrowserWindow }: any) =>
    BrowserWindow.getAllWindows()[0].setSize(1440, 1000),
  );
  const bundle = await build({
    entryPoints: [path.join(__dirname, "fixtures/figma-grid-reference.ts")],
    bundle: true,
    platform: "browser",
    format: "iife",
    globalName: "FigmaGridReference",
    write: false,
  });
  await page.evaluate(
    bundle.outputFiles[0]!.text +
      ";globalThis.FigmaGridReference=FigmaGridReference;",
  );
  // The two unrepresentable optical nodes use explicit mock exports. Their
  // transport is testable; this is not a claim to simulate Figma's optics.
  for (const n of doc.nodes.filter(
    (n) => n.name === "Glass effect" || n.name === "Texture effect",
  )) {
    n.raster = {
      bounds: {
        x: n.absoluteTransform[4],
        y: n.absoluteTransform[5],
        w: n.w,
        h: n.h,
      },
      png: await page.evaluate(({ w, h }) => {
        const c = document.createElement("canvas");
        c.width = w * 2;
        c.height = h * 2;
        const ctx = c.getContext("2d")!;
        ctx.fillStyle = "#f24d1f";
        ctx.fillRect(0, 0, c.width, c.height);
        return c.toDataURL().split(",")[1]!;
      }, n),
    };
  }
  await page.evaluate(
    ({ w, h }) => {
      const P = (window as any).PM;
      P.replaceProject(P.mkProject({ w, h, dur: 8, bg: "#272727" }));
      P.setTime(0);
    },
    { w, h },
  );
  // Exercise the exact renderer handoff used after a validated loopback send.
  // Keep this renderer audit independent of concurrent fixed-port transport tests.
  const result = await app.evaluate(
    async ({ ipcMain, BrowserWindow }: any, doc) =>
      new Promise<any>((resolve, reject) => {
        const id = "fixture-grid-import";
        const finish = (event: any, value: any) => {
          if (value?.id !== id) return;
          clearTimeout(timer);
          ipcMain.off("figma:result", finish);
          resolve(value.result);
        };
        const timer = setTimeout(() => {
          ipcMain.off("figma:result", finish);
          reject(new Error("Fixture import timed out"));
        }, 30000);
        ipcMain.on("figma:result", finish);
        BrowserWindow.getAllWindows()[0].webContents.send("figma:receive", {
          id,
          doc,
        });
      }),
    validateFigmaDoc(doc),
  );
  expect(result.error).toBeUndefined();
  expect(result.flattened).toBe(2);
  await page.evaluate(async () => {
    const P = (window as any).PM;
    P.selectLayers([]);
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
  });
  if (process.env.FIGMA_GRID_AUDIT_DIR)
    await page.screenshot({
      path:
        process.env.FIGMA_GRID_AUDIT_DIR +
        "/" +
        (process.env.FIGMA_GRID_AUDIT_PHASE ?? "review") +
        "-editor.png",
    });

  const actual = await page.evaluate(
    ({ w, h }) => (window as any).PM.renderFrameTo(0, w, h).toDataURL(),
    { w, h },
  );
  const model = await page.evaluate(
    async ({ doc, w, h }) => {
      const R = (window as any).FigmaGridReference,
        result = await R.gridReference(doc, w, h);
      const host = document.createElement("div");
      host.id = "figma-grid-reference";
      Object.assign(host.style, {
        position: "fixed",
        left: "0",
        top: "0",
        width: w + "px",
        height: h + "px",
        zIndex: "2147483647",
        background: "#272727",
      });
      host.attachShadow({ mode: "open" }).innerHTML =
        '<img style="position:absolute;left:0;top:0;width:100%;height:100%" src="' +
        result.canvas.toDataURL() +
        '">' +
        doc.nodes
          .filter((n) => n.type === "TEXT")
          .map((n) => R.textReference(n, result.offset))
          .join("");
      document.body.append(host);
      await document.fonts.ready;
      R.finishReferenceText(host, doc);
      return {
        offset: result.offset,
        types: (window as any).PM.proj.layers
          .filter((l: any) =>
            ["Blend HUE", "Blend SATURATION"].includes(l.name),
          )
          .map((l: any) => ({ name: l.name, type: l.type, on: l.on })),
      };
    },
    { doc, w, h },
  );
  const reference = await page
    .locator("#figma-grid-reference")
    .screenshot({ scale: "css" });
  await page.locator("#figma-grid-reference").evaluate((e) => e.remove());
  const report = await page.evaluate(
    async ({ actual, reference, doc, w, h, offset }) => {
      const load = async (src: string) => {
        const i = new Image();
        i.src = src;
        await i.decode();
        const c = document.createElement("canvas");
        c.width = w;
        c.height = h;
        c.getContext("2d")!.drawImage(i, 0, 0, w, h);
        return c;
      };
      const a = await load(actual),
        r = await load(reference),
        ac = a.getContext("2d")!,
        rc = r.getContext("2d")!;
      const cells = doc.nodes.map((n, i) => {
        const x = Math.round(offset.x + (i % 7) * 130),
          y = Math.round(offset.y + Math.floor(i / 7) * 110),
          av = ac.getImageData(x, y, 130, 110).data,
          rv = rc.getImageData(x, y, 130, 110).data;
        let sum = 0,
          max = 0,
          bad = 0,
          changed = 0,
          referenceChanged = 0,
          interiorMismatch = 0;
        for (let j = 0; j < av.length; j += 4) {
          let pixel = 0;
          for (let k = 0; k < 3; k++) {
            const delta = Math.abs(av[j + k]! - rv[j + k]!);
            sum += delta;
            max = Math.max(max, delta);
            pixel = Math.max(pixel, delta);
          }
          if (pixel > 16) {
            bad++;
            const x = (j / 4) % 130,
              y = Math.floor(j / 4 / 130);
            let matched = false;
            for (let dy = -1; dy <= 1 && !matched; dy++)
              for (let dx = -1; dx <= 1 && !matched; dx++) {
                if (x + dx < 0 || x + dx >= 130 || y + dy < 0 || y + dy >= 110)
                  continue;
                const ri = ((y + dy) * 130 + x + dx) * 4;
                if (
                  Math.max(
                    ...[0, 1, 2].map((k) => Math.abs(av[j + k]! - rv[ri + k]!)),
                  ) <= 16
                )
                  matched = true;
              }
            if (!matched) interiorMismatch++;
          }
          if (av[j] !== 39 || av[j + 1] !== 39 || av[j + 2] !== 39) changed++;
          if (rv[j] !== 39 || rv[j + 1] !== 39 || rv[j + 2] !== 39)
            referenceChanged++;
        }
        // CSS and Canvas use different glyph antialiasing. Compare colored
        // glyph coverage within two pixels in both directions for text, while
        // retaining the raw RGB differences in the evidence report.
        let textMismatchFraction: number | undefined;
        if (n.type === "TEXT") {
          const category = (d: Uint8ClampedArray, j: number) =>
            d[j]! > d[j + 2]! + 30 ? 1 : d[j + 2]! > d[j]! + 30 ? 2 : 0;
          let bad = 0,
            total = 0;
          for (const [source, target] of [
            [av, rv],
            [rv, av],
          ]) {
            for (let j = 0; j < source.length; j += 4) {
              const color = category(source, j);
              if (!color) continue;
              total++;
              const x = (j / 4) % 130,
                y = Math.floor(j / 4 / 130);
              let match = false;
              for (let dy = -2; dy <= 2 && !match; dy++)
                for (let dx = -2; dx <= 2 && !match; dx++) {
                  if (
                    x + dx < 0 ||
                    x + dx >= 130 ||
                    y + dy < 0 ||
                    y + dy >= 110
                  )
                    continue;
                  match =
                    category(target, ((y + dy) * 130 + x + dx) * 4) === color;
                }
              if (!match) bad++;
            }
          }
          textMismatchFraction = bad / Math.max(1, total);
        }
        let noise:
          { darkFraction: number; unclassifiedFraction: number } | undefined;
        if (n.name === "Noise effect") {
          const base = n.fills[0]!.color!,
            solid = [base.r, base.g, base.b].map((v) => Math.round(v * 255)),
            alpha = n.effects[0]!.opacity!;
          const dark = solid.map((v) => Math.round(v * (1 - alpha)));
          let darkCount = 0,
            unknown = 0,
            count = 0;
          for (let y = 22; y < 78; y++)
            for (let x = 22; x < 108; x++) {
              const j = (y * 130 + x) * 4;
              const near = (color: number[]) =>
                color.every((v, k) => Math.abs(v - av[j + k]!) <= 2);
              if (near(dark)) darkCount++;
              else if (!near(solid)) unknown++;
              count++;
            }
          noise = {
            darkFraction: darkCount / count,
            unclassifiedFraction: unknown / count,
          };
        }
        return {
          row: Math.floor(i / 7) + 1,
          column: (i % 7) + 1,
          name: n.name,
          max,
          mean: sum / (130 * 110 * 3),
          badFraction: bad / (130 * 110),
          changed,
          referenceChanged,
          interiorMismatch,
          interiorFraction:
            interiorMismatch / Math.max(1, changed, referenceChanged),
          kind:
            n.name === "Noise effect"
              ? "noise-distribution"
              : n.name === "Glass effect" || n.name === "Texture effect"
                ? "supplied-mock-export"
                : n.smoothing
                  ? "circular-outline-bound"
                  : n.type === "TEXT"
                    ? "browser-css-layout"
                    : "svg-canvas",
          noise,
          textMismatchFraction,
        };
      });
      return { cells, errors: [...(window as any).PM.GL.errors] };
    },
    {
      actual,
      reference: "data:image/png;base64," + reference.toString("base64"),
      doc,
      w,
      h,
      offset: model.offset,
    },
  );
  expect(model.types).toEqual(
    expect.arrayContaining([
      { name: "Blend HUE", type: "shape", on: true },
      { name: "Blend SATURATION", type: "shape", on: true },
    ]),
  );
  for (const n of ["Blend HUE", "Blend SATURATION"]) {
    const cell = report.cells.find((c) => c.name === n)!;
    expect(cell.changed, n + " preserves the achromatic backdrop").toBe(0);
    expect(cell.max, n + " agrees with the browser reference").toBe(0);
  }
  const colored = await page.evaluate(
    ({ w, h }) => {
      const P = (window as any).PM;
      P.proj.bg = "#345678";
      P.proj.backgroundFill = P.normalizeFill(null, "#345678");
      P.touch();
      return P.renderFrameTo(0, w, h).toDataURL();
    },
    { w, h },
  );
  const proof = await page.evaluate(
    async (args) =>
      (window as any).FigmaGridReference.blendProof(
        args.doc,
        args.actual,
        args.reference,
        args.colored,
        args.offset,
      ),
    {
      doc,
      actual,
      reference: "data:image/png;base64," + reference.toString("base64"),
      colored,
      offset: model.offset,
    },
  );
  for (const cell of proof.cells) {
    expect(
      cell.changedInterior,
      cell.name + " exists on a colored backdrop",
    ).toBeGreaterThan(4000);
    expect(
      cell.maxInterior,
      cell.name + " matches native browser blending",
    ).toBeLessThanOrEqual(1);
  }
  await info.attach("grid-reference", {
    body: reference,
    contentType: "image/png",
  });
  await info.attach("grid-actual", {
    body: Buffer.from(actual.split(",")[1]!, "base64"),
    contentType: "image/png",
  });
  await info.attach("cell-audit", {
    body: JSON.stringify(report, null, 2),
    contentType: "application/json",
  });
  const folder = process.env.FIGMA_GRID_AUDIT_DIR,
    phase = process.env.FIGMA_GRID_AUDIT_PHASE ?? "review";
  if (folder) {
    await mkdir(folder, { recursive: true });
    await writeFile(folder + "/" + phase + "-reference.png", reference);
    await writeFile(
      folder + "/" + phase + "-actual.png",
      Buffer.from(actual.split(",")[1]!, "base64"),
    );
    await writeFile(
      folder + "/" + phase + "-cells.json",
      JSON.stringify({ ...report, blendProof: proof.cells }, null, 2),
    );
    await writeFile(
      folder + "/" + phase + "-blend-proof.png",
      Buffer.from(proof.png.split(",")[1]!, "base64"),
    );
  }
  expect(report.errors).toEqual([]);
  const noise = report.cells.find((c) => c.name === "Noise effect")!.noise!;
  expect(noise.darkFraction).toBeGreaterThan(0.2);
  expect(noise.darkFraction).toBeLessThan(0.4);
  expect(noise.unclassifiedFraction).toBeLessThan(0.01);
  const material = report.cells.filter(
    (c) =>
      c.kind !== "noise-distribution" &&
      (c.kind === "browser-css-layout"
        ? c.textMismatchFraction! > 0.02
        : c.mean > 1 && c.interiorMismatch > 10 && c.interiorFraction > 0.02),
  );
  expect(
    material,
    "Material cell differences against independently authored references",
  ).toEqual([]);
});

test("rich text keeps a font baseline when the letters change between ascenders and descenders", async ({
  session,
}) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    await document.fonts.ready;
    const P = (window as any).PM;
    return ["HH", "gg", ".."].map((text) => {
      const layer = P.mkLayer("text", {
        d: {
          text,
          font: "Arial",
          size: 24,
          weight: 400,
          leading: 1.25,
          align: "left",
          richText: true,
          runs: [],
          boxWidth: 0,
          boxHeight: 0,
        },
      });
      return P.textLayout(layer, 0).characters[0].baseline;
    });
  });
  expect(result[0]).toBeGreaterThan(0);
  expect(Math.max(...result) - Math.min(...result)).toBeLessThan(0.001);
});
