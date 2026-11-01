import { afterEach, describe, expect, it, vi } from "vitest";

const id = "11111111-1111-4111-8111-111111111111";
const other = "22222222-2222-4222-8222-222222222222";
const image = (printing: string, side = "front", size = "art_crop") => `https://cards.scryfall.io/${size}/${side}/1/1/${printing}.jpg?1`;
const entry = (faces: unknown[]) => ({ oracle_id: "oracle", name: "Fixture", face_names: ["front", "back"], faces, mana_cost: "", cmc: 0, type_line: "", colors: [], color_identity: [], keywords: [] });

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });

describe("exact-image artist lookup", () => {
  it("uses matching image metadata without requesting an Oracle default printing", async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ fixture: entry([{ normal: image(id, "front", "normal"), art_crop: image(id), artist: "Exact artist" }]) })));
    vi.stubGlobal("fetch", fetch);
    const { fetchImageArtist } = await import("../scryfall.ts");
    expect(await fetchImageArtist(image(id))).toBe("Exact artist");
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it("deduplicates a printing lookup and selects its actual front/back credits", async () => {
    const fetch = vi.fn(async (url: string) => new Response(JSON.stringify(url.includes("api.scryfall.com") ? {
      card_faces: [
        { artist: "Front artist", image_uris: { normal: image(id, "front", "normal") } },
        { artist: "Back artist", image_uris: { normal: image(id, "back", "normal") } },
      ],
    } : {})));
    vi.stubGlobal("fetch", fetch);
    const { fetchImageArtist } = await import("../scryfall.ts");
    expect(await Promise.all([fetchImageArtist(image(id)), fetchImageArtist(image(id, "back"))])).toEqual(["Front artist", "Back artist"]);
    expect(fetch.mock.calls.filter(([url]) => url.includes("api.scryfall.com"))).toEqual([[`https://api.scryfall.com/cards/${id}`]]);
  });

  it("never credits the requested image with another printing's artist", async () => {
    vi.stubGlobal("fetch", vi.fn(async (url: string) => new Response(JSON.stringify(url.includes("api.scryfall.com") ? {
      artist: "Wrong printing", image_uris: { normal: image(other, "front", "normal") },
    } : {}))));
    const { fetchImageArtist } = await import("../scryfall.ts");
    expect(await fetchImageArtist(image(id))).toBeNull();
  });
});
