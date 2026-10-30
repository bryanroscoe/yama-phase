// @vitest-environment node
import path from "node:path";
import { readFileSync } from "node:fs";

import { loadConfigFromFile, type Plugin, type PluginOption } from "vite";
import { afterEach, describe, expect, it, vi } from "vitest";

const CLIENT = path.resolve(__dirname, "../../..");

/** The production build's plugins, flattened out of any nested presets. */
async function buildConfig() {
  const loaded = await loadConfigFromFile(
    { command: "build", mode: "production" },
    path.join(CLIENT, "vite.config.ts"),
    CLIENT,
    "silent",
  );
  if (!loaded) throw new Error("vite.config.ts did not load");
  const options = (loaded.config.plugins ?? []) as PluginOption[];
  const plugins = (await Promise.all(options.flat(Infinity as 1))).flat(Infinity as 1) as unknown[];
  const named = plugins.filter(
    (plugin): plugin is Plugin => typeof plugin === "object" && plugin !== null && "name" in plugin,
  );
  return { base: loaded.config.base, define: loaded.config.define ?? {}, plugins: named };
}

afterEach(() => vi.unstubAllEnvs());

function dataDefaults(hostBase: string) {
  vi.stubEnv("PHASE_BASE_PATH", hostBase);
  for (const name of ["DATA_BASE_URL", "CARD_DATA_URL", "CARD_DATA_LOCALE_URL_TEMPLATE", "SCRYFALL_IMAGES_LOCALE_URL_TEMPLATE"]) {
    vi.stubEnv(name, "");
  }
}

function expectManifestUrls(define: Record<string, unknown>, prefix: string) {
  const files = JSON.parse(readFileSync(path.resolve(CLIENT, "../data-files.json"), "utf8")) as string[];
  for (const filename of files) {
    const token = `__${filename.replace(/\.json$/, "").replace(/[.-]/g, "_").toUpperCase()}_URL__`;
    expect(JSON.parse(define[token] as string), token).toBe(`${prefix}${filename}`);
  }
}

function emittedAssets(plugin: Plugin): Array<{ type: string; fileName: string; source: string }> {
  const emitFile = vi.fn();
  const hook = plugin.generateBundle as unknown as (this: unknown) => void;
  hook.call({ emitFile });
  return emitFile.mock.calls.map(([file]) => file as { type: string; fileName: string; source: string });
}

describe("build manifest", () => {
  it.each(["/", "/board/", "/games/phase/"])("keeps local data and emitted manifests within %s", async (hostBase) => {
    dataDefaults(hostBase);
    const { base, define, plugins } = await buildConfig();
    expect(base).toBe(hostBase);
    expectManifestUrls(define, hostBase);
    expect(JSON.parse(define.__CARD_DATA_URL__ as string)).toBe(`${hostBase}card-data.json`);
    expect(JSON.parse(define.__STATUS_URL__ as string)).toBe(`${hostBase}status.json`);
    expect(JSON.parse(define.__CARD_DATA_LOCALE_URL_TEMPLATE__ as string)).toBe(`${hostBase}card-data.{lng}.json`);
    expect(JSON.parse(define.__SCRYFALL_IMAGES_LOCALE_URL_TEMPLATE__ as string)).toBe(`${hostBase}scryfall-images.v2.{lng}.json`);
    // Emitted filenames stay relative: Vite serves them under its base, alongside data.
    expect(emittedAssets(plugins.find((p) => p.name === "build-manifest")!)[0].fileName).toBe("build.json");
    expect(emittedAssets(plugins.find((p) => p.name === "offline-shell-marker")!)[0].fileName).toMatch(/^offline-shell-[^/]+\.json$/);
  });

  it("keeps a separate data CDN without moving the local card database", async () => {
    dataDefaults("/board/");
    vi.stubEnv("DATA_BASE_URL", "https://cdn.example/phase/");
    const { define } = await buildConfig();
    expectManifestUrls(define, "https://cdn.example/phase/");
    expect(JSON.parse(define.__CARD_DATA_URL__ as string)).toBe("/board/card-data.json");
    expect(JSON.parse(define.__STATUS_URL__ as string)).toBe("https://cdn.example/phase/status.json");
    expect(JSON.parse(define.__CARD_DATA_LOCALE_URL_TEMPLATE__ as string)).toBe("https://cdn.example/phase/card-data.{lng}.json");
  });

  it("preserves explicit Yama data routes and locale templates", async () => {
    dataDefaults("/board/");
    vi.stubEnv("DATA_BASE_URL", "/api/play/board-data");
    vi.stubEnv("CARD_DATA_URL", "/api/play/board-data/card-data.json");
    vi.stubEnv("CARD_DATA_LOCALE_URL_TEMPLATE", "https://cdn.example/content/{lng}.json");
    vi.stubEnv("SCRYFALL_IMAGES_LOCALE_URL_TEMPLATE", "/local-images/{lng}.json");
    const { define } = await buildConfig();
    expectManifestUrls(define, "/api/play/board-data/");
    expect(JSON.parse(define.__CARD_DATA_URL__ as string)).toBe("/api/play/board-data/card-data.json");
    expect(JSON.parse(define.__CARD_DATA_LOCALE_URL_TEMPLATE__ as string)).toBe("https://cdn.example/content/{lng}.json");
    expect(JSON.parse(define.__SCRYFALL_IMAGES_LOCALE_URL_TEMPLATE__ as string)).toBe("/local-images/{lng}.json");
  });

  it("emits /build.json carrying the bundle's __BUILD_HASH__", async () => {
    const { define, plugins } = await buildConfig();
    // Positive control for the traversal: the sibling marker plugin is found
    // by the same lookup.
    expect(plugins.find((plugin) => plugin.name === "offline-shell-marker")).toBeDefined();

    const manifest = plugins.find((plugin) => plugin.name === "build-manifest");
    expect(manifest).toBeDefined();
    expect(manifest!.apply).toBe("build");

    const assets = emittedAssets(manifest!);
    expect(assets).toHaveLength(1);
    expect(assets[0].type).toBe("asset");
    expect(assets[0].fileName).toBe("build.json");
    expect(JSON.parse(assets[0].source)).toEqual({
      build: JSON.parse(define.__BUILD_HASH__ as string),
    });
  });

  it("enables __IS_PREVIEW_BUILD__ when ENGINE_FINGERPRINT or PREVIEW_BUILD is set", async () => {
    const prevFingerprint = process.env.ENGINE_FINGERPRINT;
    const prevPreview = process.env.PREVIEW_BUILD;
    try {
      delete process.env.ENGINE_FINGERPRINT;
      delete process.env.PREVIEW_BUILD;
      expect((await buildConfig()).define.__IS_PREVIEW_BUILD__).toBe("false");

      process.env.ENGINE_FINGERPRINT = "0123456789abcdef";
      expect((await buildConfig()).define.__IS_PREVIEW_BUILD__).toBe("true");

      delete process.env.ENGINE_FINGERPRINT;
      process.env.PREVIEW_BUILD = "true";
      expect((await buildConfig()).define.__IS_PREVIEW_BUILD__).toBe("true");
    } finally {
      if (prevFingerprint === undefined) delete process.env.ENGINE_FINGERPRINT;
      else process.env.ENGINE_FINGERPRINT = prevFingerprint;
      if (prevPreview === undefined) delete process.env.PREVIEW_BUILD;
      else process.env.PREVIEW_BUILD = prevPreview;
    }
  });
});
