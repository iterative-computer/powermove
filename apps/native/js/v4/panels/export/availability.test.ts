import { expect, test } from "bun:test";
import availability from "../../../../crates/promedia/tests/export_availability.json";
import {
  alphaOptions,
  audioOptions,
  codecOptions,
  selectedCodec,
  selectedAlpha,
  shows,
  type Format,
  type Profile,
  type VideoCodec,
} from "./spec";
import { fixture } from "./fixtures";

test("sheet availability matches the engine-checked delivery table", () => {
  for (const row of availability) {
    const options = row.options as {
      format: Format;
      profile: Profile;
      videoCodec?: VideoCodec;
    };
    expect(alphaOptions(options).map((o): string => o.v)).toEqual(
      row.alphaModes,
    );
    expect(
      ["prores", "mp4", "webm"].includes(options.format)
        ? audioOptions(options).map((o): string => o.v)
        : [],
    ).toEqual(row.audioCodings);
    expect(shows(options, "audioCoding")).toBe(row.audioCodings.length > 1);
    expect(shows(options, "alphaMode")).toBe(row.alphaModes.length > 1);
    expect(shows(options, "alpha")).toBe(row.alphaModes.length > 1);
    expect(shows(options, "quality")).toBe(
      !!options.videoCodec && options.videoCodec !== "prores",
    );
    expect(shows(options, "timecode")).toBe(row.timecode);
    if (options.videoCodec)
      expect(
        codecOptions(options).some((o) => o.v === options.videoCodec),
      ).toBe(true);
  }
});

test("legacy codecs and dormant alpha open as the same delivery", () => {
  expect(selectedCodec({ format: "prores", codec: "hevc" })).toBe("prores");
  expect(selectedCodec({ format: "mp4", codec: "hevc" })).toBe("hevc10");
  expect(selectedCodec({ format: "webm", codec: "hevc" })).toBe("vp9");
  expect(selectedAlpha({ format: "webm", alpha: true })).toBe("none");
  expect(selectedAlpha({ format: "prores", profile: "422", alpha: true })).toBe(
    "none",
  );
  expect(
    selectedAlpha({ format: "prores", profile: "4444", alpha: true }),
  ).toBe("straight");
});

test("codec changes keep alpha valid; HDR only offers HDR codecs", () => {
  const { model } = fixture();
  model.set("format", "prores");
  model.set("videoCodec", "hevc");
  model.set("alphaMode", "premultiplied");
  expect(model.options.alphaMode).toBe("premultiplied");
  model.set("videoCodec", "h264");
  expect(model.options.alphaMode).toBe("none");
  expect(() => model.set("alphaMode", "straight")).toThrow();
  expect(() => model.set("videoCodec", "vp8")).toThrow();
  expect(
    codecOptions({ format: "mp4", dynamicRange: "pq" }).map((o) => o.v),
  ).toEqual(["hevc10"]);
});
