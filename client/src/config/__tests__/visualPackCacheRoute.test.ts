// @vitest-environment node
import { runInNewContext } from "node:vm";
import { afterEach, describe, expect, it, vi } from "vitest";
import { visualPackCacheRoute } from "../visualPackCacheRoute";
import { syntheticCachePath } from "../../services/visualPacks/browser/records";
import type { CatalogRoot } from "../../services/visualPacks/types";

afterEach(() => vi.unstubAllEnvs());

describe("hosted visual-pack cache URLs", () => {
  it.each(["/", "/board/", "/games/phase/"])("matches installed image paths under %s", (base) => {
    vi.stubEnv("BASE_URL", base);
    const origin = "https://host.example";
    const scope = `${origin}${base}`;
    // Like Workbox, evaluate the serialized matcher without its original
    // module scope. This also catches accidental closure dependencies.
    const matches = runInNewContext(`(${visualPackCacheRoute.toString()})`, {
      URL,
      registration: { scope },
    }) as typeof visualPackCacheRoute;
    const object = "a".repeat(64) as CatalogRoot;
    for (const media of ["image/jpeg", "image/svg+xml"] as const) {
      const path = syntheticCachePath(object, media);
      expect(path.startsWith(`${base}__visual-packs/sha256/`)).toBe(true);
      expect(matches({ sameOrigin: true, url: new URL(path, origin) })).toBe(true);
      expect(matches({ sameOrigin: false, url: new URL(path, "https://other.example") })).toBe(false);
    }
    for (const suffix of [`${object}.txt`, "short.jpg", `${object}.jpg/extra`]) {
      expect(matches({ sameOrigin: true, url: new URL(`${base}__visual-packs/sha256/${suffix}`, origin) })).toBe(false);
    }
    if (base !== "/") {
      expect(matches({ sameOrigin: true, url: new URL(`/__visual-packs/sha256/${object}.jpg`, origin) })).toBe(false);
      expect(matches({ sameOrigin: true, url: new URL(`${base.slice(0, -1)}walk/__visual-packs/sha256/${object}.jpg`, origin) })).toBe(false);
    }
  });
});
