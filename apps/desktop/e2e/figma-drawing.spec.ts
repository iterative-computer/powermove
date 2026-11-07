import { writeFile } from "node:fs/promises";
import { test, expect } from "./helpers/app";
import { allFeatures } from "../src/shared/figma/__fixtures__/all-features";
import type { FigmaNode } from "../src/shared/figma/doc";

test.use({
  desktopLaunchOptions: { env: { POWERMOVE_TEST_FIGMA_AUTOSTART: "1" } },
});
async function setup(session: any) {
  await session.openEditor();
  const page = session.page;
  await session.app.evaluate(({ Menu }: any) =>
    Menu.getApplicationMenu().getMenuItemById("connectFigma").click(),
  );
  await expect(
    page.getByRole("button", { name: "Copy Plugin Path" }),
  ).toBeEnabled();
  const pair = fetch("http://localhost:47832/pair", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ code: "654321" }),
  });
  await page.getByRole("button", { name: "Allow", exact: true }).click();
  const { token } = (await (await pair).json()) as any;
  await page.getByRole("button", { name: "Done", exact: true }).click();
  return async (nodes: FigmaNode[]) => {
    await page.evaluate(() => {
      const P = (window as any).PM;
      P.replaceProject(P.mkProject({ w: 160, h: 120, dur: 8, bg: "#272727" }));
      P.setTime(0);
    });
    const r = await fetch("http://localhost:47832/send", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: "Bearer " + token,
      },
      body: JSON.stringify({ ...allFeatures(), nodes }),
    });
    expect(r.status, await r.text()).toBe(200);
  };
}
test("renders every editable fixture through the compositor without covering unrelated pixels", async ({
  session,
}) => {
  test.setTimeout(90000);
  const send = await setup(session),
    fixture = allFeatures();
  for (const original of fixture.nodes.filter(
    (n) => !["Glass effect", "Texture effect"].includes(n.name),
  )) {
    const n = structuredClone(original);
    n.absoluteTransform = [1, 0, 0, 1, 0, 0];
    if (n.children.length) continue;
    await send([n]);
    if (n.name.startsWith("Blend "))
      await session.page.evaluate(() => {
        const P = (window as any).PM;
        P.proj.bg = "#345678";
        P.proj.backgroundFill = P.normalizeFill(null, "#345678");
      });
    const p = await session.page.evaluate(() => {
      const P = (window as any).PM;
      P.rasterClear();
      const c = P.renderFrameTo(0, 160, 120),
        ctx = c.getContext("2d"),
        a = ctx.getImageData(0, 0, 160, 120).data;
      let changed = 0;
      const bg = Array.from(ctx.getImageData(0, 0, 1, 1).data);
      for (let y = 15; y < 105; y++)
        for (let x = 20; x < 140; x++) {
          const i = (y * 160 + x) * 4;
          if (
            Math.abs(a[i] - Number(bg[0])) +
              Math.abs(a[i + 1] - Number(bg[1])) +
              Math.abs(a[i + 2] - Number(bg[2])) >
            15
          )
            changed++;
        }
      return {
        bg,
        corner: Array.from(ctx.getImageData(159, 119, 1, 1).data),
        changed,
        errors: Array.from(P.GL.errors.entries()),
      };
    });
    expect(p.errors, n.name).toEqual([]);
    expect(p.corner, n.name).toEqual(p.bg);
    // A background-only blur over a uniform backdrop may paint no changed pixels.
    expect(p.changed, n.name).toBeGreaterThan(
      n.name === "Background blur" ? 0 : 8,
    );
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
  expect(
    session.diagnostics.console.filter(
      (c: any) => c.type === "error" && c.text.includes("failed to render"),
    ),
  ).toEqual([]);
});

test("matches SVG reference gradients, compound holes, open strokes and ellipse arcs", async ({
  session,
}, testInfo) => {
  test.setTimeout(90000);
  const send = await setup(session),
    fixture = allFeatures(),
    results: any[] = [];
  for (const name of [
    "LINEAR gradient",
    "RADIAL gradient",
    "ANGULAR gradient",
    "DIAMOND gradient",
    "Compound holes",
    "Open curved path",
    "Arc donut",
    "INSIDE stroke",
    "OUTSIDE stroke",
    "CENTER stroke",
    "Inner shadow",
    "Multiple spread shadows",
    "Layer blur",
  ]) {
    const n = structuredClone(fixture.nodes.find((n) => n.name === name)!);
    n.absoluteTransform = [1, 0, 0, 1, 0, 0];
    await send([n]);
    const proof = await session.page.evaluate(async (n) => {
      const P = (window as any).PM;
      if (n.effects.length) {
        for (const l of P.proj.layers) if (!l.group) l.p["position.y"].v -= 15;
        P.touch();
      }
      const c = P.renderFrameTo(0, 160, 120),
        actual = c.getContext("2d").getImageData(0, 0, 160, 120).data;
      const fmt = (c: any) =>
        `rgba(${c.r * 255},${c.g * 255},${c.b * 255},${c.a ?? 1})`;
      let body = "",
        defs = "";
      const p = n.fills[0];
      if (p?.type === "GRADIENT_DIAMOND" || p?.type === "GRADIENT_ANGULAR") {
        const cv = document.createElement("canvas");
        cv.width = 90;
        cv.height = 60;
        const ctx = cv.getContext("2d")!,
          m = p.gradientTransform!;
        if (p.type === "GRADIENT_ANGULAR") {
          const det = m[0] * m[3] - m[1] * m[2];
          ctx.scale(90, 60);
          ctx.transform(
            m[3] / det,
            -m[1] / det,
            -m[2] / det,
            m[0] / det,
            (m[2] * m[5] - m[3] * m[4]) / det,
            (m[1] * m[4] - m[0] * m[5]) / det,
          );
          const g = ctx.createConicGradient(0, 0.5, 0.5);
          for (const s of p.gradientStops!)
            g.addColorStop(s.position, fmt(s.color));
          ctx.fillStyle = g;
          ctx.fillRect(-4, -4, 8, 8);
        } else {
          const image = ctx.createImageData(90, 60),
            stops = p.gradientStops!;
          for (let y = 0; y < 60; y++)
            for (let x = 0; x < 90; x++) {
              const u = (x + 0.5) / 90,
                v = (y + 0.5) / 60,
                t = Math.min(
                  1,
                  Math.max(
                    0,
                    2 *
                      (Math.abs(m[0] * u + m[2] * v + m[4] - 0.5) +
                        Math.abs(m[1] * u + m[3] * v + m[5] - 0.5)),
                  ),
                );
              let i = 1;
              while (i < stops.length - 1 && stops[i]!.position < t) i++;
              const a = stops[i - 1]!,
                b = stops[i]!,
                f = (t - a.position) / (b.position - a.position),
                alpha = (a.color.a ?? 1) * (1 - f) + (b.color.a ?? 1) * f;
              image.data.set(
                [
                  ...(["r", "g", "b"] as const).map((k) =>
                    alpha
                      ? (255 *
                          (a.color[k] * (a.color.a ?? 1) * (1 - f) +
                            b.color[k] * (b.color.a ?? 1) * f)) /
                        alpha
                      : 0,
                  ),
                  255 * alpha,
                ],
                (y * 90 + x) * 4,
              );
            }
          ctx.putImageData(image, 0, 0);
        }
        body = `<image width="90" height="60" href="${cv.toDataURL()}"/>`;
      } else if (p?.type.startsWith("GRADIENT")) {
        const m = p.gradientTransform!,
          det = m[0] * m[3] - m[1] * m[2],
          inv = [
            m[3] / det,
            -m[1] / det,
            -m[2] / det,
            m[0] / det,
            (m[2] * m[5] - m[3] * m[4]) / det,
            (m[1] * m[4] - m[0] * m[5]) / det,
          ];
        const tag =
          p.type === "GRADIENT_RADIAL" ? "radialGradient" : "linearGradient";
        defs = `<${tag} id="g" gradientTransform="matrix(${inv.join(" ")})">${p.gradientStops!.map((s) => `<stop offset="${s.position}" stop-color="${fmt(s.color)}"/>`).join("")}</${tag}>`;
        body = '<rect width="90" height="60" fill="url(#g)"/>';
      } else if (n.effects.length) {
        defs =
          '<filter id="f" x="-100%" y="-100%" width="300%" height="300%" color-interpolation-filters="sRGB">';
        const effects: string[] = [];
        for (const [i, e] of n.effects.entries()) {
          if (e.type === "LAYER_BLUR") {
            defs += `<feGaussianBlur stdDeviation="${(e.radius ?? 0) / 2}"/>`;
            continue;
          }
          const inner = e.type === "INNER_SHADOW",
            a = "a" + i,
            b = "b" + i,
            c = "c" + i;
          if (inner)
            defs += `<feComponentTransfer in="SourceAlpha" result="${a}"><feFuncA type="table" tableValues="1 0"/></feComponentTransfer>`;
          defs += `<feMorphology in="${inner ? a : "SourceAlpha"}" operator="dilate" radius="${e.spread ?? 0}" result="${b}"/><feOffset in="${b}" dx="${e.offset?.x ?? 0}" dy="${e.offset?.y ?? 0}"/><feGaussianBlur stdDeviation="${(e.radius ?? 0) / 2}" result="${c}"/><feFlood flood-color="${fmt(e.color)}"/><feComposite in2="${c}" operator="in" result="shadow${i}"/>`;
          if (inner)
            defs += `<feComposite in2="SourceAlpha" operator="in" result="shadow${i}"/>`;
          effects.push("shadow" + i);
        }
        if (effects.length)
          defs += `<feMerge>${n.effects[0]!.type === "INNER_SHADOW" ? '<feMergeNode in="SourceGraphic"/>' : ""}${[
            ...effects,
          ]
            .reverse()
            .map((e) => `<feMergeNode in="${e}"/>`)
            .join(
              "",
            )}${n.effects[0]!.type !== "INNER_SHADOW" ? '<feMergeNode in="SourceGraphic"/>' : ""}</feMerge>`;
        defs += "</filter>";
        body = '<rect width="90" height="60" fill="#f24d1f" filter="url(#f)"/>';
      } else if (n.strokes.length && !n.paths?.length) {
        const outside = n.strokeAlign === "OUTSIDE",
          inside = n.strokeAlign === "INSIDE";
        defs =
          '<clipPath id="clip"><rect width="90" height="60"/></clipPath><mask id="outside" x="-100%" y="-100%" width="300%" height="300%"><rect x="-90" y="-60" width="270" height="180" fill="white"/><rect width="90" height="60" fill="black"/></mask>';
        body = `<rect width="90" height="60" fill="#f24d1f"/><rect width="90" height="60" fill="none" stroke="#3399ff" stroke-width="${(n.strokeWeight ?? 0) * (inside || outside ? 2 : 1)}" ${inside ? 'clip-path="url(#clip)"' : outside ? 'mask="url(#outside)"' : ""}/>`;
      } else if (n.paths?.length) {
        body = n.paths
          .map(
            (p) =>
              `<path d="${p.data}" fill="${n.fills.length ? "#f24d1f" : "none"}" fill-rule="${p.windingRule === "EVENODD" ? "evenodd" : "nonzero"}" stroke="${n.strokes.length ? "#f24d1f" : "none"}" stroke-width="${n.strokeWeight ?? 0}" stroke-linecap="${(n.cap ?? "butt").toLowerCase()}"/>`,
          )
          .join("");
      } else if (n.arc) {
        const a = n.arc,
          x = 45,
          y = 30,
          pt = (t: number, r = 1) =>
            `${x + x * r * Math.cos(t)} ${y + y * r * Math.sin(t)}`,
          large = a.end - a.start > Math.PI ? 1 : 0;
        body = `<path fill="#f24d1f" d="M${pt(a.start)} A45 30 0 ${large} 1 ${pt(a.end)} L${pt(a.end, a.inner)} A${45 * a.inner} ${30 * a.inner} 0 ${large} 0 ${pt(a.start, a.inner)} Z"/>`;
      }
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="160" height="120"><rect width="160" height="120" fill="#272727"/>${defs}<g transform="translate(35 ${n.effects.length ? 15 : 30})">${body}</g></svg>`;
      const image = new Image();
      image.src = "data:image/svg+xml;base64," + btoa(svg);
      await image.decode();
      const ref = document.createElement("canvas");
      ref.width = 160;
      ref.height = 120;
      const ctx = ref.getContext("2d")!;
      ctx.drawImage(image, 0, 0);
      const expected = ctx.getImageData(0, 0, 160, 120).data;
      let max = 0,
        sum = 0,
        count = 0;
      for (let i = 0; i < actual.length; i += 4)
        for (let k = 0; k < 3; k++) {
          const delta = Math.abs(actual[i + k] - expected[i + k]);
          max = Math.max(max, delta);
          sum += delta;
          count++;
        }
      return {
        max,
        mean: sum / count,
        actual: c.toDataURL().split(",")[1],
        reference: ref.toDataURL().split(",")[1],
      };
    }, n);
    results.push({ name, max: proof.max, mean: proof.mean });
    await testInfo.attach(name + " actual", {
      body: Buffer.from(proof.actual, "base64"),
      contentType: "image/png",
    });
    await testInfo.attach(name + " reference", {
      body: Buffer.from(proof.reference, "base64"),
      contentType: "image/png",
    });
    expect(proof.mean, name).toBeLessThan(5);
  }
  if (process.env.FIGMA_DIFF_REPORT)
    await writeFile(
      process.env.FIGMA_DIFF_REPORT,
      JSON.stringify(results, null, 2),
    );
  await testInfo.attach("pixel-differences", {
    body: JSON.stringify(results, null, 2),
    contentType: "application/json",
  });
});

test("intersects nested clips and luminance mattes without a fixed eight-mask depth", async ({
  session,
}) => {
  const send = await setup(session),
    base = allFeatures().nodes[0]!;
  const leaf: FigmaNode = {
    ...base,
    id: "leaf",
    name: "leaf",
    type: "RECTANGLE",
    w: 100,
    h: 70,
    absoluteTransform: [1, 0, 0, 1, 0, 0],
    fills: [{ type: "SOLID", color: { r: 0, g: 1, b: 0 } }],
    children: [],
  };
  let root = leaf;
  for (let i = 0; i < 10; i++)
    root = {
      ...base,
      id: "clip" + i,
      name: "clip" + i,
      type: "FRAME",
      w: 60 + i * 2,
      h: 50,
      absoluteTransform: [1, 0, 0, 1, 0, 0],
      clipsContent: true,
      fills: [],
      children: [root],
    };
  await send([root]);
  const p = await session.page.evaluate(() => {
    const P = (window as any).PM,
      c = P.renderFrameTo(0, 160, 120),
      ctx = c.getContext("2d");
    return {
      center: Array.from(ctx.getImageData(80, 60, 1, 1).data),
      outside: Array.from(ctx.getImageData(130, 60, 1, 1).data),
      images: P.proj.layers.filter((l: any) => l.type === "image").length,
    };
  });
  expect(p.images).toBe(0);
  expect(p.center).toEqual([0, 255, 0, 255]);
  expect(p.outside).toEqual([39, 39, 39, 255]);
});

test("new compositor blend modes match the browser reference, including alpha", async ({
  session,
}) => {
  const send = await setup(session),
    base = allFeatures().nodes[0]!,
    differences: any[] = [];
  for (const [figma, canvasMode] of Object.entries({
    SOFT_LIGHT: "soft-light",
    COLOR_BURN: "color-burn",
    COLOR_DODGE: "color-dodge",
    HARD_LIGHT: "hard-light",
    EXCLUSION: "exclusion",
    HUE: "hue",
    SATURATION: "saturation",
    COLOR: "color",
    LUMINOSITY: "luminosity",
    LINEAR_BURN: "linearburn",
  })) {
    const node = {
      ...base,
      name: figma,
      absoluteTransform: [1, 0, 0, 1, 0, 0],
      blendMode: figma,
      opacity: 0.7,
      fills: [{ type: "SOLID", color: { r: 0.9, g: 0.3, b: 0.2, a: 0.8 } }],
    } as FigmaNode;
    await send([node]);
    const diff = await session.page.evaluate((mode) => {
      const P = (window as any).PM;
      P.proj.bg = "#345678";
      P.proj.backgroundFill = P.normalizeFill(null, "#345678");
      const actual = Array.from(
        P.renderFrameTo(0, 160, 120).getContext("2d").getImageData(80, 60, 1, 1)
          .data,
      ) as number[];
      const cv = document.createElement("canvas");
      cv.width = cv.height = 1;
      const c = cv.getContext("2d")!;
      c.fillStyle = "#345678";
      c.fillRect(0, 0, 1, 1);
      if (mode === "linearburn") {
        const b = [52, 86, 120],
          s = [0.9, 0.3, 0.2].map((v) => Math.round(v * 255)),
          a = 0.56;
        return Math.max(
          ...b.map((v, i) =>
            Math.abs(
              actual[i] - (v * (1 - a) + Math.max(0, v + s[i] - 255) * a),
            ),
          ),
        );
      }
      c.globalCompositeOperation = mode as GlobalCompositeOperation;
      c.globalAlpha = 0.7;
      c.fillStyle = "rgba(230,77,51,.8)";
      c.fillRect(0, 0, 1, 1);
      const expected = c.getImageData(0, 0, 1, 1).data;
      return Math.max(
        ...actual.slice(0, 3).map((v, i) => Math.abs(v - expected[i])),
      );
    }, canvasMode);
    differences.push({ feature: figma, max: diff });
    expect(diff, figma).toBeLessThanOrEqual(3);
  }
  if (process.env.FIGMA_DIFF_REPORT)
    await writeFile(
      process.env.FIGMA_DIFF_REPORT + ".blends.json",
      JSON.stringify(differences, null, 2),
    );
});

test("image fill, fit, crop and tile preserve the asset and match a clipped Canvas reference", async ({
  session,
}) => {
  const send = await setup(session),
    fixture = allFeatures(),
    results: any[] = [];
  for (const mode of ["FILL", "FIT", "CROP", "TILE"]) {
    const n = structuredClone(
      fixture.nodes.find((n) => n.name === "Image " + mode)!,
    );
    n.absoluteTransform = [1, 0, 0, 1, 0, 0];
    await send([n]);
    const diff = await session.page.evaluate(
      async ({ mode, png }) => {
        const P = (window as any).PM,
          actual = P.renderFrameTo(0, 160, 120)
            .getContext("2d")
            .getImageData(0, 0, 160, 120).data;
        const cv = document.createElement("canvas");
        cv.width = 160;
        cv.height = 120;
        const c = cv.getContext("2d")!;
        c.fillStyle = "#272727";
        c.fillRect(0, 0, 160, 120);
        c.translate(35, 30);
        c.beginPath();
        c.roundRect(0, 0, 90, 60, 12);
        c.clip();
        const image = new Image();
        image.src = "data:image/png;base64," + png;
        await image.decode();
        if (mode === "TILE") {
          const p = c.createPattern(image, "repeat")!;
          p.setTransform(new DOMMatrix([0.5, 0, 0, 0.5, 0, 0]));
          c.fillStyle = p;
          c.fillRect(0, 0, 90, 60);
        } else if (mode === "CROP") {
          c.drawImage(image, -9, 0, 90, 60);
        } else {
          const scale =
            mode === "FIT"
              ? Math.min(90 / 16, 60 / 12)
              : Math.max(90 / 16, 60 / 12);
          c.drawImage(
            image,
            (90 - 16 * scale) / 2,
            (60 - 12 * scale) / 2,
            16 * scale,
            12 * scale,
          );
        }
        const expected = c.getImageData(0, 0, 160, 120).data;
        let max = 0,
          sum = 0;
        for (let i = 0; i < actual.length; i += 4)
          for (let k = 0; k < 3; k++) {
            const delta = Math.abs(actual[i + k] - expected[i + k]);
            max = Math.max(max, delta);
            sum += delta;
          }
        return {
          max,
          mean: sum / (160 * 120 * 3),
          assets: Object.keys(P.proj.assets).length,
          images: P.proj.layers
            .filter((l: any) => l.type === "image")
            .map((l: any) => l.d.imageMapping.mode),
        };
      },
      { mode, png: fixture.images.checker! },
    );
    results.push({ feature: "Image " + mode, max: diff.max, mean: diff.mean });
    expect(diff.assets).toBe(1);
    expect(diff.images).toEqual([mode.toLowerCase()]);
    expect(diff.mean, mode).toBeLessThan(3);
  }
  if (process.env.FIGMA_DIFF_REPORT)
    await writeFile(
      process.env.FIGMA_DIFF_REPORT + ".images.json",
      JSON.stringify(results, null, 2),
    );
});

test("alpha, luminance and vector masks retain their distinct coverage, including group masks", async ({
  session,
}) => {
  const send = await setup(session),
    base = allFeatures().nodes[0]!;
  for (const type of ["ALPHA", "LUMINANCE", "VECTOR"])
    for (const group of [false, true]) {
      const mask: FigmaNode = {
        ...base,
        id: "mask",
        name: "mask",
        type: group ? "GROUP" : "ELLIPSE",
        absoluteTransform: [1, 0, 0, 1, 0, 0],
        isMask: true,
        maskType: type,
        opacity: 0.5,
        fills: group
          ? []
          : [{ type: "SOLID", color: { r: 0.5, g: 0.5, b: 0.5, a: 0.25 } }],
        children: group
          ? [
              {
                ...base,
                id: "coverage",
                name: "coverage",
                type: "ELLIPSE",
                absoluteTransform: [1, 0, 0, 1, 0, 0],
                fills: [
                  { type: "SOLID", color: { r: 0.5, g: 0.5, b: 0.5, a: 0.25 } },
                ],
                children: [],
              },
            ]
          : [],
      };
      const content: FigmaNode = {
        ...base,
        id: "paint",
        name: "paint",
        absoluteTransform: [1, 0, 0, 1, 0, 0],
        fills: [{ type: "SOLID", color: { r: 1, g: 0, b: 0 } }],
        children: [],
      };
      await send([mask, content]);
      const pixel = await session.page.evaluate(() => {
        const P = (window as any).PM;
        return Array.from(
          P.renderFrameTo(0, 160, 120)
            .getContext("2d")
            .getImageData(80, 60, 1, 1).data,
        ) as number[];
      });
      const alpha = type === "VECTOR" ? 1 : type === "ALPHA" ? 0.125 : 0.0625,
        expected = [
          255 * alpha + 39 * (1 - alpha),
          39 * (1 - alpha),
          39 * (1 - alpha),
        ];
      expect(
        Math.max(
          ...pixel.slice(0, 3).map((v, i) => Math.abs(v - expected[i]!)),
        ),
        type + " " + group,
      ).toBeLessThan(3);
    }
});

test("a frame clip intersects its child mask instead of replacing it", async ({
  session,
}) => {
  const send = await setup(session),
    base = allFeatures().nodes[0]!;
  const root: FigmaNode = {
    ...base,
    id: "frame",
    name: "frame",
    type: "FRAME",
    absoluteTransform: [1, 0, 0, 1, 0, 0],
    fills: [],
    clipsContent: true,
    children: [
      {
        ...base,
        id: "mask",
        name: "mask",
        type: "ELLIPSE",
        absoluteTransform: [1, 0, 0, 1, -20, 0],
        isMask: true,
        maskType: "ALPHA",
        fills: [{ type: "SOLID", color: { r: 1, g: 1, b: 1 } }],
        children: [],
      },
      {
        ...base,
        id: "paint",
        name: "paint",
        w: 150,
        absoluteTransform: [1, 0, 0, 1, -40, 0],
        fills: [{ type: "SOLID", color: { r: 1, g: 0, b: 0 } }],
        children: [],
      },
    ],
  };
  await send([root]);
  const proof = await session.page.evaluate(() => {
    const P = (window as any).PM,
      c = P.renderFrameTo(0, 160, 120),
      ctx = c.getContext("2d");
    return {
      pixels: [20, 80, 120].map((x) =>
        Array.from(ctx.getImageData(x, 60, 1, 1).data),
      ),
      intersections: P.proj.layers.find((l: any) => l.name === "paint")
        .matteSources?.length,
    };
  });
  expect(proof.intersections).toBe(1);
  expect(proof.pixels).toEqual([
    [39, 39, 39, 255],
    [255, 0, 0, 255],
    [39, 39, 39, 255],
  ]);
});

test("background blur inside an isolated clipped group samples the composition below it", async ({
  session,
}) => {
  const send = await setup(session),
    base = allFeatures().nodes[0]!;
  const paint = (r: number, g: number, b: number) => [
    { type: "SOLID", color: { r, g, b } },
  ];
  const background: FigmaNode = {
    ...base,
    id: "background",
    name: "background",
    absoluteTransform: [1, 0, 0, 1, 0, 0],
    w: 90,
    h: 60,
    radii: [0, 0, 0, 0],
    fills: paint(1, 0, 0),
    children: [],
  };
  const foreground: FigmaNode = {
    ...base,
    id: "foreground",
    name: "foreground",
    absoluteTransform: [1, 0, 0, 1, 45, 0],
    w: 45,
    h: 60,
    radii: [0, 0, 0, 0],
    fills: paint(0, 0, 1),
    children: [],
  };
  const glass: FigmaNode = {
    ...base,
    id: "blur",
    name: "blur",
    absoluteTransform: [1, 0, 0, 1, 15, 10],
    w: 60,
    h: 40,
    radii: [0, 0, 0, 0],
    fills: [],
    effects: [{ type: "BACKGROUND_BLUR", radius: 12 }],
    children: [],
  };
  const group: FigmaNode = {
    ...base,
    id: "group",
    name: "group",
    type: "FRAME",
    blendMode: "NORMAL",
    absoluteTransform: [1, 0, 0, 1, 0, 0],
    w: 90,
    h: 60,
    clipsContent: true,
    radii: [0, 0, 0, 0],
    fills: [],
    effects: [],
    children: [glass],
  };
  await send([background, foreground, group]);
  const p = await session.page.evaluate(() => {
    const P = (window as any).PM,
      c = P.renderFrameTo(0, 160, 120),
      x = c.getContext("2d");
    return [70, 79, 80, 89, 10].map(
      (px) => Array.from(x.getImageData(px, 60, 1, 1).data) as number[],
    );
  });
  expect(p[1]![0]).toBeGreaterThan(60);
  expect(p[1]![2]).toBeGreaterThan(60);
  expect(p[2]![0]).toBeGreaterThan(60);
  expect(p[2]![2]).toBeGreaterThan(60);
  expect(p[0]![0]).toBeGreaterThan(p[3]![0]!);
  expect(p[4]).toEqual([39, 39, 39, 255]);
});

test("rich text strokes, gradients and animators paint editable glyphs without losing run metrics", async ({
  session,
}) => {
  const send = await setup(session),
    base = allFeatures().nodes.find((n) => n.name === "Styled text")!;
  const n = structuredClone(base);
  n.absoluteTransform = [1, 0, 0, 1, 0, 0];
  n.w = 90;
  n.h = 30;
  n.text!.characters = "AV fi";
  n.text!.autoResize = "WIDTH_AND_HEIGHT";
  n.text!.runs = [
    {
      ...n.text!.runs[0]!,
      start: 0,
      end: 5,
      size: 24,
      leading: 30,
      fills: [{ type: "SOLID", color: { r: 1, g: 0, b: 0 } }],
    },
  ];
  n.fills = [{ type: "SOLID", color: { r: 1, g: 0, b: 0 } }];
  n.text!.runs[0]!.fills = [
    {
      type: "GRADIENT_LINEAR",
      gradientTransform: [1, 0, 0, 1, 0, 0],
      gradientStops: [
        { position: 0, color: { r: 1, g: 0, b: 0 } },
        { position: 1, color: { r: 0, g: 0, b: 1 } },
      ],
    },
  ];
  n.strokes = [{ type: "SOLID", color: { r: 0, g: 1, b: 0 } }];
  n.strokeWeight = 3;
  n.strokeAlign = "OUTSIDE";
  await send([n]);
  const proof = await session.page.evaluate(() => {
    const P = (window as any).PM,
      l = P.proj.layers.find((l: any) => l.type === "text");
    const count = () => {
      P.rasterClear();
      const c = P.renderFrameTo(P.time, 160, 120),
        a = c.getContext("2d").getImageData(0, 0, 160, 120).data;
      let red = 0,
        green = 0;
      for (let i = 0; i < a.length; i += 4) {
        if (a[i] > 100 && a[i] > a[i + 1] * 2) red++;
        if (a[i + 1] > 100 && a[i + 1] > a[i] * 2) green++;
      }
      return { red, green };
    };
    const still = count(),
      layout = P.textLayout(l);
    l.d.animators = [
      {
        id: "move",
        name: "Move",
        mode: "wave",
        unit: "characters",
        p: Object.fromEntries(
          Object.entries({ amount: 100, speed: 1, spread: 0, y: 2 }).map(
            ([k, v]) => [k, P.P(v)],
          ),
        ),
      },
    ];
    P.touch();
    P.setTime(0.25);
    const moving = count();
    return {
      still,
      moving,
      lines: layout.lines.length,
      errors: [...P.GL.errors],
    };
  });
  expect(proof.lines).toBe(1);
  expect(proof.still.red).toBeGreaterThan(20);
  expect(proof.still.green).toBeGreaterThan(20);
  expect(proof.moving.red).toBeGreaterThan(20);
  expect(proof.moving.green).toBeGreaterThan(20);
  expect(proof.errors).toEqual([]);
});

test("Figma normal groups isolate child blends while pass-through groups blend with lower layers", async ({
  session,
}) => {
  const send = await setup(session),
    base = allFeatures().nodes[0]!;
  const values: number[][] = [];
  for (const mode of ["NORMAL", "PASS_THROUGH"]) {
    const bg: FigmaNode = {
      ...base,
      id: "background",
      name: "background",
      absoluteTransform: [1, 0, 0, 1, 0, 0],
      fills: [{ type: "SOLID", color: { r: 1, g: 0, b: 0 } }],
      children: [],
    };
    const child: FigmaNode = {
      ...base,
      id: "child",
      name: "child",
      absoluteTransform: [1, 0, 0, 1, 0, 0],
      blendMode: "MULTIPLY",
      fills: [{ type: "SOLID", color: { r: 0, g: 0, b: 1 } }],
      children: [],
    };
    const group: FigmaNode = {
      ...base,
      id: "group",
      name: "group",
      type: "GROUP",
      absoluteTransform: [1, 0, 0, 1, 0, 0],
      blendMode: mode,
      fills: [],
      children: [child],
    };
    await send([bg, group]);
    values.push(
      await session.page.evaluate(() => {
        const P = (window as any).PM;
        return Array.from(
          P.renderFrameTo(0, 160, 120)
            .getContext("2d")
            .getImageData(80, 60, 1, 1).data,
        );
      }),
    );
  }
  expect(values).toEqual([
    [0, 0, 255, 255],
    [0, 0, 0, 255],
  ]);
});

test("linear burn works inside styled text paint stacks without flattening the text", async ({
  session,
}) => {
  const send = await setup(session),
    n = structuredClone(
      allFeatures().nodes.find((n) => n.name === "Styled text")!,
    );
  n.absoluteTransform = [1, 0, 0, 1, 0, 0];
  n.w = 90;
  n.h = 60;
  n.fills = [];
  n.text!.characters = "M";
  n.text!.autoResize = "WIDTH_AND_HEIGHT";
  n.text!.runs = [
    {
      start: 0,
      end: 1,
      font: "Arial",
      style: "Regular",
      size: 48,
      leading: 60,
      fills: [
        {
          type: "SOLID",
          blendMode: "LINEAR_BURN",
          color: { r: 120 / 255, g: 80 / 255, b: 200 / 255 },
        },
        { type: "SOLID", color: { r: 150 / 255, g: 100 / 255, b: 60 / 255 } },
      ],
    },
  ];
  await send([n]);
  const p = await session.page.evaluate(() => {
    const P = (window as any).PM,
      c = P.renderFrameTo(0, 160, 120),
      data = c.getContext("2d").getImageData(0, 0, 160, 120).data;
    let matched = 0;
    for (let i = 0; i < data.length; i += 4)
      if (
        Math.abs(data[i] - 15) < 2 &&
        data[i + 1] < 2 &&
        Math.abs(data[i + 2] - 5) < 2
      )
        matched++;
    return {
      matched,
      types: P.proj.layers.map((l: any) => l.type),
      errors: [...P.GL.errors],
    };
  });
  expect(p.types).toEqual(["text"]);
  expect(p.matched).toBeGreaterThan(50);
  expect(p.errors).toEqual([]);
});

test("native kiwi paste preserves compound holes and embedded image assets", async ({
  session,
}) => {
  await session.openEditor();
  await session.app.evaluate(({ BrowserWindow }: any) => {
    BrowserWindow.getAllWindows()[0].isFocused = () => true;
  });
  const fixture = allFeatures();
  fixture.nodes = ["Compound holes", "Image CROP"].map((name) =>
    fixture.nodes.find((n) => n.name === name)!,
  );
  fixture.nodes.forEach(
    (n, i) => (n.absoluteTransform = [1, 0, 0, 1, i * 110, 0]),
  );
  const { clipboardFixture } = await import("./fixtures/figma-all");
  const html = await clipboardFixture(fixture);
  await session.page.evaluate(() => {
    const P = (window as any).PM;
    P.replaceProject(P.mkProject({ w: 240, h: 120, dur: 8, bg: "#272727" }));
    (document.activeElement as HTMLElement)?.blur();
  });
  await session.app.evaluate(
    async ({ clipboard, ClipboardItem }: any, html) =>
      clipboard.write([new ClipboardItem({ "text/html": html })]),
    html,
  );
  await session.page.keyboard.press("Meta+V");
  await expect
    .poll(() =>
      session.page.evaluate(() => (window as any).PM.proj.layers.length),
    )
    .toBeGreaterThan(0);
  const result = await session.page.evaluate(() => {
    const P = (window as any).PM,
      c = P.renderFrameTo(0, 240, 120),
      ctx = c.getContext("2d");
    return {
      hole: Array.from(ctx.getImageData(55, 60, 1, 1).data),
      edge: Array.from(ctx.getImageData(25, 35, 1, 1).data),
      rasters: P.proj.layers.filter(
        (l: any) => l.type === "image" && !l.d.imageMapping,
      ).length,
      images: P.proj.layers.filter((l: any) => l.type === "image").length,
      assets: Object.keys(P.proj.assets).length,
    };
  });
  expect(result.hole).toEqual([39, 39, 39, 255]);
  expect(result.edge[0]).toBeGreaterThan(230);
  expect(result.rasters).toBe(0);
  expect(result.images).toBe(1);
  expect(result.assets).toBe(1);
});
