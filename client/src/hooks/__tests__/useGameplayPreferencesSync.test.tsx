import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { EngineAdapter, GameState, GameplayPreferences } from "../../adapter/types";
import { dispatchActionForGameSession } from "../../game/dispatch";
import { useGameplayPreferencesSync } from "../useGameplayPreferencesSync";
import {
  nextGameSessionGeneration,
  useGameStore,
} from "../../stores/gameStore";
import { usePreferencesStore } from "../../stores/preferencesStore";
import { useUiStore } from "../../stores/uiStore";

vi.mock("../../game/dispatch", () => ({ dispatchActionForGameSession: vi.fn() }));

const adapter = {} as EngineAdapter;
const gameState = {} as GameState;

describe("useGameplayPreferencesSync", () => {
  beforeEach(() => {
    vi.mocked(dispatchActionForGameSession).mockClear();
    usePreferencesStore.setState({
      phaseStops: [],
      priorityPassingMode: "Standard",
    });
    useGameStore.setState({
      adapter: null,
      gameState: null,
      engineCommitEpoch: 0,
      gameSessionGeneration: nextGameSessionGeneration(),
    });
    useUiStore.setState({ fullControl: false });
  });

  afterEach(() => cleanup());

  it("adopts each reconnect envelope on the same lifecycle without sending it back", async () => {
    const server = { initialGameplayPreferences: {
      phase_stops: [], priority_passing_mode: "Standard", full_control: false,
    } as GameplayPreferences };
    const serverAdapter = server as unknown as EngineAdapter;
    useGameStore.setState({ adapter: serverAdapter, gameState, engineCommitEpoch: 1 });
    const generation = useGameStore.getState().gameSessionGeneration;
    renderHook(() => useGameplayPreferencesSync());

    act(() => {
      server.initialGameplayPreferences = {
        phase_stops: [{ phase: "Draw", scope: "OwnTurn" }],
        priority_passing_mode: "SkipLowUseWindows", full_control: true,
      };
      useGameStore.setState({ engineCommitEpoch: 2 });
    });
    expect(useGameStore.getState().gameSessionGeneration).toBe(generation);
    expect(usePreferencesStore.getState().phaseStops).toEqual([{ phase: "Draw", scope: "OwnTurn" }]);
    expect(usePreferencesStore.getState().priorityPassingMode).toBe("SkipLowUseWindows");
    expect(useUiStore.getState().fullControl).toBe(true);
    expect(dispatchActionForGameSession).not.toHaveBeenCalled();

    act(() => useUiStore.getState().toggleFullControl());
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(1));
    expect(dispatchActionForGameSession).toHaveBeenLastCalledWith(
      { type: "SetPriorityPassingMode", data: { mode: "SkipLowUseWindows" } }, serverAdapter, generation,
    );
  });

  it("drops an old preference completion before adopting a newer reconnect envelope", async () => {
    const server = { initialGameplayPreferences: {
      phase_stops: [], priority_passing_mode: "Standard", full_control: false,
    } as GameplayPreferences };
    const serverAdapter = server as unknown as EngineAdapter;
    useGameStore.setState({ adapter: serverAdapter, gameState, engineCommitEpoch: 1 });
    renderHook(() => useGameplayPreferencesSync());
    let finish!: () => void;
    vi.mocked(dispatchActionForGameSession).mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
    act(() => usePreferencesStore.getState().setPhaseStops([{ phase: "End", scope: "AllTurns" }]));
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(1));
    act(() => {
      // Both a pending mode change and an older stops response must lose to
      // the accepted reconnect, even though the adapter/generation stay put.
      usePreferencesStore.getState().setPriorityPassingMode("SkipLowUseWindows");
      server.initialGameplayPreferences = {
        phase_stops: [{ phase: "Upkeep", scope: "OwnTurn" }],
        priority_passing_mode: "Standard", full_control: true,
      };
      useGameStore.setState({ engineCommitEpoch: 2 });
    });
    await act(async () => { finish(); });
    expect(usePreferencesStore.getState().phaseStops).toEqual([{ phase: "Upkeep", scope: "OwnTurn" }]);
    expect(usePreferencesStore.getState().priorityPassingMode).toBe("Standard");
    expect(useUiStore.getState().fullControl).toBe(true);
    expect(dispatchActionForGameSession).toHaveBeenCalledTimes(1);
  });

  it("adopts server preferences without dispatching bootstrap actions, then sends explicit changes", async () => {
    const serverAdapter = {
      initialGameplayPreferences: {
        phase_stops: [{ phase: "Upkeep", scope: "OpponentsTurns" }],
        priority_passing_mode: "Standard",
        full_control: true,
      },
    } as unknown as EngineAdapter;
    useGameStore.setState({ adapter: serverAdapter, gameState, engineCommitEpoch: 1 });
    const hook = renderHook(() => useGameplayPreferencesSync());

    expect(usePreferencesStore.getState().phaseStops).toEqual([{ phase: "Upkeep", scope: "OpponentsTurns" }]);
    expect(useUiStore.getState().fullControl).toBe(true);
    expect(dispatchActionForGameSession).not.toHaveBeenCalled();
    hook.unmount();
    renderHook(() => useGameplayPreferencesSync());
    act(() => {
      useGameStore.setState({ engineCommitEpoch: 2 });
      usePreferencesStore.setState({ cardSize: "large" });
    });
    expect(dispatchActionForGameSession).not.toHaveBeenCalled();

    act(() => usePreferencesStore.getState().setPhaseStops([]));
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(1));
    expect(dispatchActionForGameSession).toHaveBeenLastCalledWith(
      { type: "SetPhaseStops", data: { stops: [] } }, serverAdapter, expect.any(Number),
    );
    act(() => useUiStore.getState().toggleFullControl());
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(2));
    expect(dispatchActionForGameSession).toHaveBeenLastCalledWith(
      { type: "SetPriorityPassingMode", data: { mode: "Standard" } }, serverAdapter, expect.any(Number),
    );
    // A genuinely new attachment adopts the host again, even on the same adapter.
    act(() => useGameStore.setState({ gameSessionGeneration: nextGameSessionGeneration() }));
    expect(useUiStore.getState().fullControl).toBe(true);
    expect(usePreferencesStore.getState().phaseStops).toEqual([{ phase: "Upkeep", scope: "OpponentsTurns" }]);
    expect(dispatchActionForGameSession).toHaveBeenCalledTimes(2);
  });

  it("preserves temporary server holds and pass modes when unrelated preferences change", async () => {
    const serverAdapter = {
      initialGameplayPreferences: { phase_stops: [], priority_passing_mode: "Standard", full_control: false },
    } as unknown as EngineAdapter;
    useUiStore.setState({ fullControl: true });
    usePreferencesStore.getState().setPriorityPassingMode("SkipLowUseWindows");
    useGameStore.setState({ adapter: serverAdapter, gameState, engineCommitEpoch: 1 });
    renderHook(() => useGameplayPreferencesSync());
    expect(useUiStore.getState().fullControl).toBe(false);
    expect(usePreferencesStore.getState().priorityPassingMode).toBe("Standard");
    expect(dispatchActionForGameSession).not.toHaveBeenCalled();

    act(() => usePreferencesStore.getState().setPhaseStops([{ phase: "End", scope: "OwnTurn" }]));
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(1));
    expect(dispatchActionForGameSession).toHaveBeenLastCalledWith(
      { type: "SetPhaseStops", data: { stops: [{ phase: "End", scope: "OwnTurn" }] } }, serverAdapter, expect.any(Number),
    );
    act(() => usePreferencesStore.getState().setPriorityPassingMode("SkipLowUseWindows"));
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(2));
    expect(dispatchActionForGameSession).toHaveBeenLastCalledWith(
      { type: "SetPriorityPassingMode", data: { mode: "SkipLowUseWindows" } }, serverAdapter, expect.any(Number),
    );
  });

  it("keeps a read-only server attachment from sending local defaults", () => {
    const spectatorAdapter = { initialGameplayPreferences: null } as unknown as EngineAdapter;
    useGameStore.setState({ adapter: spectatorAdapter, gameState, engineCommitEpoch: 1 });
    renderHook(() => useGameplayPreferencesSync());
    expect(dispatchActionForGameSession).not.toHaveBeenCalled();
    act(() => useUiStore.getState().toggleFullControl());
    expect(dispatchActionForGameSession).not.toHaveBeenCalled();
  });

  it("waits for an installed snapshot, dedupes, and resends for a fresh lifecycle", async () => {
    renderHook(() => useGameplayPreferencesSync());

    act(() => {
      useGameStore.setState({ adapter });
    });
    expect(dispatchActionForGameSession).not.toHaveBeenCalled();

    act(() => {
      useGameStore.setState({ gameState, engineCommitEpoch: 1 });
    });
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(2));
    expect(dispatchActionForGameSession).toHaveBeenNthCalledWith(
      1,
      { type: "SetPhaseStops", data: { stops: [] } },
      adapter,
      expect.any(Number),
    );
    expect(dispatchActionForGameSession).toHaveBeenNthCalledWith(
      2,
      { type: "SetPriorityPassingMode", data: { mode: "Standard" } },
      adapter,
      expect.any(Number),
    );

    act(() => {
      useGameStore.setState({ engineCommitEpoch: 2 });
    });
    expect(dispatchActionForGameSession).toHaveBeenCalledTimes(2);

    act(() => {
      usePreferencesStore.getState().setPhaseStops([]);
      usePreferencesStore.getState().setPriorityPassingMode("Standard");
    });
    expect(dispatchActionForGameSession).toHaveBeenCalledTimes(2);

    act(() => usePreferencesStore.getState().setPriorityPassingMode("SkipLowUseWindows"));
    await waitFor(() => {
      expect(dispatchActionForGameSession).toHaveBeenLastCalledWith(
        { type: "SetPriorityPassingMode", data: { mode: "SkipLowUseWindows" } },
        adapter,
        expect.any(Number),
      );
      expect(dispatchActionForGameSession).toHaveBeenCalledTimes(3);
    });

    const firstGeneration = useGameStore.getState().gameSessionGeneration;
    act(() => {
      useGameStore.setState({ gameSessionGeneration: nextGameSessionGeneration() });
    });
    expect(useGameStore.getState().gameSessionGeneration).toBeGreaterThan(firstGeneration);
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(5));

    const secondGeneration = useGameStore.getState().gameSessionGeneration;
    act(() => {
      useGameStore.setState({ gameSessionGeneration: nextGameSessionGeneration() });
    });
    expect(useGameStore.getState().gameSessionGeneration).toBeGreaterThan(secondGeneration);
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(7));
  });

  it("retries rejected sends without duplicating successful preferences", async () => {
    vi.mocked(dispatchActionForGameSession)
      .mockRejectedValueOnce(new Error("transient"))
      .mockResolvedValue(undefined);
    useGameStore.setState({ adapter, gameState, engineCommitEpoch: 1 });
    renderHook(() => useGameplayPreferencesSync());

    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(1));

    act(() => usePreferencesStore.getState().setPhaseStops([]));
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(3));
    expect(dispatchActionForGameSession).toHaveBeenNthCalledWith(
      2,
      { type: "SetPhaseStops", data: { stops: [] } },
      adapter,
      expect.any(Number),
    );
    expect(dispatchActionForGameSession).toHaveBeenNthCalledWith(
      3,
      { type: "SetPriorityPassingMode", data: { mode: "Standard" } },
      adapter,
      expect.any(Number),
    );

    act(() => usePreferencesStore.getState().setPhaseStops([]));
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(3));
  });

  it("overrides the synced mode while Full Control is on, and restores it when off", async () => {
    useGameStore.setState({ adapter, gameState, engineCommitEpoch: 1 });
    renderHook(() => useGameplayPreferencesSync());
    await waitFor(() => expect(dispatchActionForGameSession).toHaveBeenCalledTimes(2));
    expect(dispatchActionForGameSession).toHaveBeenLastCalledWith(
      { type: "SetPriorityPassingMode", data: { mode: "Standard" } },
      adapter,
      expect.any(Number),
    );

    // Full Control lives in `uiStore`, not `preferencesStore`, so it reaches the
    // engine only through its own subscription.
    act(() => useUiStore.getState().toggleFullControl());
    await waitFor(() => {
      expect(dispatchActionForGameSession).toHaveBeenLastCalledWith(
        { type: "SetPriorityPassingMode", data: { mode: "FullControl" } },
        adapter,
        expect.any(Number),
      );
    });

    // An unrelated `uiStore` write must not re-dispatch: the subscription is a
    // plain (non-selector) one guarded on the previous `fullControl` value.
    const callsAfterOverride = vi.mocked(dispatchActionForGameSession).mock.calls.length;
    act(() => useUiStore.setState({ selectedCardIds: [] }));
    expect(dispatchActionForGameSession).toHaveBeenCalledTimes(callsAfterOverride);

    act(() => useUiStore.getState().toggleFullControl());
    await waitFor(() => {
      expect(dispatchActionForGameSession).toHaveBeenLastCalledWith(
        { type: "SetPriorityPassingMode", data: { mode: "Standard" } },
        adapter,
        expect.any(Number),
      );
    });
  });
});
