import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useGameStore } from "../../stores/gameStore";
import { buildFormatConfig, buildGameState } from "../../test/factories/gameStateFactory";

const { flushNow, installTelemetryLifecycle, trackEvent } = vi.hoisted(() => ({
  flushNow: vi.fn(),
  installTelemetryLifecycle: vi.fn(),
  trackEvent: vi.fn(),
}));

vi.mock("../telemetry", () => ({
  flushNow,
  installTelemetryLifecycle,
  trackEvent,
}));

import { coarseRoute, installTelemetry, reportBoundaryError } from "../telemetryEvents";

describe("hosted telemetry routes", () => {
  let originalUrl: string;

  beforeEach(() => {
    originalUrl = window.location.href;
    trackEvent.mockClear();
  });

  afterEach(() => {
    window.history.replaceState(null, "", originalUrl);
    vi.unstubAllEnvs();
  });

  it.each([
    ["/board/", "/board/game/private-id?token=private#choice", "/game/private-id", "/game"],
    ["/board/", "/board/draft/private-pod", "/draft/private-pod", "/draft"],
    ["/board/", "/board/", "/", "/"],
    ["/board/", "/board", "/", "/"],
    ["/games/phase/", "/games/phase/replay?save=private", "/replay", "/replay"],
    ["/", "/game/private-id", "/game/private-id", "/game"],
    ["/board/", "/boardwalk/private-id", "/boardwalk/private-id", "/boardwalk"],
    ["/game/", "/game/game/private-id", "/game/private-id", "/game"],
    ["/board/", "/board/board/private-id", "/board/private-id", "/board"],
  ])("uses the same route vocabulary for global and router paths at %s%s", (base, url, routerPath, route) => {
    vi.stubEnv("BASE_URL", base);
    window.history.replaceState(null, "", url);
    expect(coarseRoute()).toBe(route);
    expect(coarseRoute(routerPath)).toBe(route);
    reportBoundaryError(new Error("test error"));
    expect(trackEvent).toHaveBeenCalledWith("js_error", expect.objectContaining({ route }));
  });
});

describe("telemetry game event tracking", () => {
  beforeEach(() => {
    useGameStore.getState().reset();
    vi.clearAllMocks();
    installTelemetry();
    trackEvent.mockClear();
  });

  it("reports native mode when it is committed before the first game snapshot", () => {
    useGameStore.setState({
      gameId: "engine-mode-telemetry",
      gameMode: "ai",
      engineMode: "native",
      aiSeatIds: [1],
    });
    useGameStore.setState({
      gameState: buildGameState({
        format_config: buildFormatConfig({ format: "Commander" }),
        turn_number: 12,
      }),
    });

    expect(trackEvent).toHaveBeenCalledWith(
      "game_start",
      expect.objectContaining({
        engine_mode: "native",
      }),
    );
  });

  it("reports the store's engine mode and fallback reason when a game ends", () => {
    useGameStore.setState({
      gameId: "engine-mode-telemetry-fallback",
      gameMode: "ai",
      engineMode: "wasm",
      nativeEngineFallbackReason: "native_engine_unavailable",
      aiSeatIds: [1],
    });
    useGameStore.setState({
      gameState: buildGameState({
        format_config: buildFormatConfig({ format: "Commander" }),
        turn_number: 12,
      }),
    });
    useGameStore.setState({ waitingFor: { type: "GameOver", data: { winner: 0 } } });

    expect(trackEvent).toHaveBeenCalledWith(
      "game_start",
      expect.objectContaining({
        engine_mode: "wasm",
        native_fallback_reason: "native_engine_unavailable",
      }),
    );
    expect(trackEvent).toHaveBeenCalledWith(
      "game_end",
      expect.objectContaining({
        engine_mode: "wasm",
        native_fallback_reason: "native_engine_unavailable",
      }),
    );
  });
});

describe("js_error top_frame", () => {
  beforeEach(() => {
    trackEvent.mockClear();
  });

  function reportedTopFrame(stack: string): unknown {
    const error = new TypeError("boom");
    error.stack = stack;
    reportBoundaryError(error);
    return trackEvent.mock.calls[0][1].top_frame;
  }

  it.each([
    [
      "V8 (Chrome/Edge/desktop)",
      "TypeError: boom\n    at render (https://phase-rs.dev/assets/index-a1.js:12:345)\n    at commit (https://phase-rs.dev/assets/index-a1.js:9:8)",
      "at render (https://phase-rs.dev/assets/index-a1.js:12:345)",
    ],
    [
      "SpiderMonkey (Firefox)",
      "render@https://phase-rs.dev/assets/index-a1.js:12:345\ncommit@https://phase-rs.dev/assets/index-a1.js:9:8\n",
      "render@https://phase-rs.dev/assets/index-a1.js:12:345",
    ],
    [
      "JavaScriptCore (Safari) anonymous frame",
      "@https://phase-rs.dev/assets/index-a1.js:12:345\nglobal code@https://phase-rs.dev/assets/index-a1.js:1:1",
      "@https://phase-rs.dev/assets/index-a1.js:12:345",
    ],
    [
      "JavaScriptCore (Safari) native frame",
      "parse@[native code]\nload@https://phase-rs.dev/assets/index-a1.js:12:345",
      "parse@[native code]",
    ],
  ])("reports the first frame of a %s stack", (_engine, stack, expected) => {
    expect(reportedTopFrame(stack)).toBe(expected);
  });

  it("does not read a V8 header whose message contains an at-sign location as a frame", () => {
    expect(
      reportedTopFrame(
        "Error: bad url user@host.example:12:34\n    at load (https://phase-rs.dev/assets/index-a1.js:1:2)",
      ),
    ).toBe("at load (https://phase-rs.dev/assets/index-a1.js:1:2)");
  });

  it("omits top_frame when the stack has no frames", () => {
    expect(reportedTopFrame("TypeError: boom")).toBeUndefined();
  });
});
