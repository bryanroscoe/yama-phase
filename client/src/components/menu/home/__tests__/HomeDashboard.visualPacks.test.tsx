import * as scryfall from "../../../../services/scryfall.ts";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router";

import {
  ACTIVE_DECK_KEY,
  RANDOM_DECK_SELECTION,
  STORAGE_KEY_PREFIX,
} from "../../../../constants/storage";
import { HomeDashboard } from "../HomeDashboard";

const { useCardImage } = vi.hoisted(() => {
  Object.defineProperty(globalThis, "__COVERAGE_SUMMARY_URL__", {
    configurable: true,
    value: "/coverage-summary.json",
  });
  return { useCardImage: vi.fn() };
});

vi.mock("../../../../hooks/useCardImage", () => ({ useCardImage }));
vi.mock("../../../../hooks/useResumables", () => ({
  useResumables: () => ({
    match: null,
    matchSummary: null,
    quickDraft: null,
    pod: null,
    resumeMatch: vi.fn(),
  }),
}));
vi.mock("../../../../stores/cardDataStore", () => ({
  useCardDataStore: (selector: (state: { status: string }) => unknown) => selector({ status: "ready" }),
}));
vi.mock("../../../../services/deckCompatibility", () => ({
  evaluateDeckCompatibility: vi.fn().mockResolvedValue({
    color_identity: ["U"],
    color_distribution: [],
  }),
}));

function renderDashboard() {
  return render(
    <MemoryRouter>
      <HomeDashboard />
    </MemoryRouter>,
  );
}

describe("HomeDashboard visual packs", () => {
  beforeEach(() => {
    localStorage.clear();
    useCardImage.mockReset();
    useCardImage.mockReturnValue({ src: null, isLoading: false });
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 404 })));
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });

  it("passes a saved representative printing and advances its installed source", async () => {
    const advanceFailedSource = vi.fn();
    useCardImage.mockReturnValue({
      src: "visual-pack://installed/home-cover",
      isLoading: false,
      advanceFailedSource,
    });
    localStorage.setItem(ACTIVE_DECK_KEY, "Printed Deck");
    localStorage.setItem(`${STORAGE_KEY_PREFIX}Printed Deck`, JSON.stringify({
      main: [{
        name: "Opt",
        count: 4,
        sourcePrinting: { setCode: "DAR", collectorNumber: "60" },
      }],
      sideboard: [],
    }));

    renderDashboard();

    expect(await screen.findByText("Printed Deck")).toBeInTheDocument();
    expect(useCardImage).toHaveBeenCalledWith("Opt", {
      size: "art_crop",
      sourcePrinting: { setCode: "DAR", collectorNumber: "60" },
    });
    const cover = document.querySelector('img[src="visual-pack://installed/home-cover"]');
    expect(cover).not.toBeNull();
    fireEvent.error(cover!);
    expect(advanceFailedSource).toHaveBeenCalledWith("visual-pack://installed/home-cover");
  });

  it("names hosted representative art when its credit cannot be resolved", async () => {
    vi.stubEnv("VITE_HOSTED_CARD_ART", "1");
    vi.spyOn(scryfall, "fetchImageArtist").mockResolvedValue(null);
    useCardImage.mockReturnValue({ src: "unknown-printing.jpg", isLoading: false });
    localStorage.setItem(ACTIVE_DECK_KEY, "Printed Deck");
    localStorage.setItem(`${STORAGE_KEY_PREFIX}Printed Deck`, JSON.stringify({
      main: [{ name: "Opt", count: 4 }], sideboard: [],
    }));
    renderDashboard();
    expect(await screen.findByLabelText("Opt")).toBeInTheDocument();
    expect(document.querySelector('img[src="unknown-printing.jpg"]')).toBeNull();
  });

  it("does not fabricate printing identity for random selection", async () => {
    localStorage.setItem(ACTIVE_DECK_KEY, RANDOM_DECK_SELECTION);

    renderDashboard();

    expect(await screen.findByText("Random Deck")).toBeInTheDocument();
    expect(useCardImage).toHaveBeenCalledWith("", {
      size: "art_crop",
      sourcePrinting: undefined,
    });
  });
});
