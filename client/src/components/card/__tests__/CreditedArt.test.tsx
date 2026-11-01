import { rememberInstalledImageSource } from "../../../services/visualPacks/imageAttribution.ts";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CreditedArt } from "../CreditedArt.tsx";

const fetchArtist = vi.hoisted(() => vi.fn<(src: string) => Promise<string | null>>());
vi.mock("../../../services/scryfall.ts", () => ({ fetchImageArtist: fetchArtist }));

beforeEach(() => { vi.stubEnv("VITE_HOSTED_CARD_ART", "1"); fetchArtist.mockReset(); });
afterEach(() => { cleanup(); vi.unstubAllEnvs(); });

describe("hosted illustration credits", () => {
  it("pairs the displayed image with its credit and removes a stale credit immediately", async () => {
    let finish!: (artist: string) => void;
    fetchArtist.mockResolvedValueOnce("Front artist").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const view = render(<CreditedArt src="https://cards.test/front.jpg" alt="Front" className="art" />);
    await screen.findByText("Front artist");
    expect(view.container.querySelector("img")?.getAttribute("src")).toBe("https://cards.test/front.jpg");
    view.rerender(<CreditedArt src="https://cards.test/back.jpg" alt="Back" className="art" />);
    expect(screen.queryByText("Front artist")).toBeNull();
    expect(view.container.querySelector("img")).toBeNull();
    await act(async () => { finish("Back artist"); });
    expect(screen.getByText("Back artist")).toBeTruthy();
    expect(view.container.querySelector("img")?.getAttribute("src")).toBe("https://cards.test/back.jpg");
  });

  it("ignores late attribution for a replaced image", async () => {
    let finish!: (artist: string) => void;
    fetchArtist.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; })).mockResolvedValueOnce("Current artist");
    const view = render(<CreditedArt src="https://cards.test/old.jpg" alt="Old" className="art" />);
    view.rerender(<CreditedArt src="https://cards.test/current.jpg" alt="Current" className="art" />);
    await screen.findByText("Current artist");
    await act(async () => { finish("Old artist"); });
    expect(screen.queryByText("Old artist")).toBeNull();
    expect(screen.getByText("Current artist")).toBeTruthy();
  });

  it("keeps an identifiable text tile when attribution is unavailable", async () => {
    fetchArtist.mockResolvedValue(null);
    const unavailable = vi.fn();
    const view = render(<CreditedArt src="https://cards.test/unknown.jpg" alt="Unknown illustration" className="art" onAttributionUnavailable={unavailable} />);
    await act(async () => {});
    expect(view.container.querySelector("img")).toBeNull();
    expect(screen.getByLabelText("Unknown illustration")).toBeTruthy();
    expect(unavailable).toHaveBeenCalledTimes(1);
  });

  it("drops an installed image credit immediately when its source becomes ambiguous", async () => {
    const src = "/board/__visual-packs/sha256/ambiguous.jpg";
    rememberInstalledImageSource(src, "https://cards.test/front.jpg");
    fetchArtist.mockResolvedValueOnce("Original artist").mockResolvedValueOnce(null);
    const view = render(<CreditedArt src={src} alt="Card" />);
    await screen.findByText("Original artist");
    act(() => rememberInstalledImageSource(src, "https://cards.test/other-printing.jpg"));
    expect(screen.queryByText("Original artist")).toBeNull();
    expect(view.container.querySelector("img")).toBeNull();
    await act(async () => {});
    expect(fetchArtist).toHaveBeenCalledWith("https://cards.test/front.jpg");
    expect(fetchArtist).toHaveBeenLastCalledWith(src);
  });

  it("names a decorative image's fallback without changing its image alt", async () => {
    fetchArtist.mockResolvedValue(null);
    const view = render(<CreditedArt src="unknown.jpg" alt="" fallbackName="Representative card" />);
    await act(async () => {});
    expect(view.container.querySelector("img")).toBeNull();
    expect(screen.getByLabelText("Representative card")).toBeTruthy();
  });

  it("retains the ordinary Phase image path outside hosted builds", () => {
    vi.stubEnv("VITE_HOSTED_CARD_ART", "0");
    const view = render(<CreditedArt src="image.jpg" alt="Card" className="art" />);
    expect(view.container.querySelector("img")?.getAttribute("src")).toBe("image.jpg");
    expect(fetchArtist).not.toHaveBeenCalled();
  });

  it("keeps account avatars separate from card illustration attribution", () => {
    const view = render(<CreditedArt attribution="external" src="https://avatar.test/person.jpg" alt="Player" />);
    expect(view.container.querySelector("img")?.getAttribute("src")).toBe("https://avatar.test/person.jpg");
    expect(fetchArtist).not.toHaveBeenCalled();
  });
});
