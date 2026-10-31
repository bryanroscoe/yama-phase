import { useEffect } from "react";

import type {
  EngineAdapter,
  PhaseStop,
  PriorityPassingMode,
} from "../adapter/types";
import { dispatchActionForGameSession } from "../game/dispatch";
import { useGameStore } from "../stores/gameStore";
import { usePreferencesStore } from "../stores/preferencesStore";
import { useUiStore } from "../stores/uiStore";

/**
 * The mode the engine must hold for this player.
 *
 * CR 117.1: Full Control is a standing refusal to give up any priority window,
 * so it has to be engine state, not a frontend flag. An auto-pass session
 * another player installed (Resolve All) is driven inside the engine's own
 * priority loop and never consults a client, so a purely local toggle could not
 * stop it. It stays a per-session `uiStore` toggle in the UI — this only
 * projects it onto the synced preference while it is on.
 */
function effectivePriorityPassingMode(): PriorityPassingMode {
  return useUiStore.getState().fullControl
    ? "FullControl"
    : usePreferencesStore.getState().priorityPassingMode;
}

type LastSent = {
  adapter: EngineAdapter;
  generation: number;
  hostPreferences?: EngineAdapter["initialGameplayPreferences"];
  stops?: readonly PhaseStop[];
  mode?: PriorityPassingMode;
};

// Module-scoped so React StrictMode remounts cannot resend preferences for the
// same live engine lifecycle. `gameSessionGeneration` is monotonically unique,
// so a genuine init/resume/reset always invalidates this cache even when both
// the adapter object and game id are reused.
let lastSent: LastSent | null = null;
let syncRequested = false;
let syncInFlight = false;

function phaseStopsEqual(a: readonly PhaseStop[], b: readonly PhaseStop[]): boolean {
  return a.length === b.length
    && a.every((value, index) =>
      value.phase === b[index]?.phase && value.scope === b[index]?.scope,
    );
}

function isCurrentSession(adapter: EngineAdapter, generation: number, hostPreferences: EngineAdapter["initialGameplayPreferences"]): boolean {
  const game = useGameStore.getState();
  return (
    game.adapter === adapter
    && game.gameSessionGeneration === generation
    && game.gameState !== null
    && adapter.initialGameplayPreferences === hostPreferences
  );
}

function successfulSendFor(adapter: EngineAdapter, generation: number): LastSent {
  if (lastSent?.adapter === adapter && lastSent.generation === generation) {
    return lastSent;
  }
  return { adapter, generation };
}

function initializeServerPreferences(adapter: EngineAdapter, generation: number): void {
  const initial = adapter.initialGameplayPreferences;
  if (lastSent?.adapter === adapter && lastSent.generation === generation && lastSent.hostPreferences === initial) return;
  if (initial == null) return;

  // Cache before hydrating: store subscriptions may request another drain.
  // Neither attachment nor StrictMode remounts are player preference changes.
  // Each accepted reconnect replaces the host object and must hydrate again.
  lastSent = {
    adapter,
    generation,
    hostPreferences: initial,
    stops: initial.phase_stops.slice(),
    mode: initial.full_control ? "FullControl" : initial.priority_passing_mode,
  };
  usePreferencesStore.setState({
    phaseStops: initial.phase_stops.slice(),
    priorityPassingMode: initial.priority_passing_mode,
  });
  useUiStore.setState({ fullControl: initial.full_control });
}

async function drainGameplayPreferenceSync(): Promise<void> {
  if (syncInFlight) return;
  syncInFlight = true;

  try {
    while (syncRequested) {
      syncRequested = false;

      const {
        adapter,
        gameSessionGeneration: generation,
        gameState,
      } = useGameStore.getState();
      if (!adapter || !gameState) continue;
      if (adapter.initialGameplayPreferences === null) continue;

      initializeServerPreferences(adapter, generation);
      const hostPreferences = adapter.initialGameplayPreferences;
      const stops = usePreferencesStore.getState().phaseStops;
      const mode = effectivePriorityPassingMode();
      const sent = successfulSendFor(adapter, generation);

      if (!sent.stops || !phaseStopsEqual(sent.stops, stops)) {
        try {
          await dispatchActionForGameSession(
            { type: "SetPhaseStops", data: { stops: [...stops] } },
            adapter,
            generation,
          );
        } catch {
          // dispatchAction reports engine failures. Leave this value unsent so
          // the next store notification can retry it.
          if (!isCurrentSession(adapter, generation, hostPreferences)) syncRequested = true;
          continue;
        }

        const currentStops = usePreferencesStore.getState().phaseStops;
        if (isCurrentSession(adapter, generation, hostPreferences) && phaseStopsEqual(currentStops, stops)) {
          lastSent = {
            ...successfulSendFor(adapter, generation),
            stops: stops.slice(),
          };
        } else {
          syncRequested = true;
          continue;
        }
      }

      if (!isCurrentSession(adapter, generation, hostPreferences)) {
        syncRequested = true;
        continue;
      }

      const currentMode = effectivePriorityPassingMode();
      if (currentMode !== mode) {
        syncRequested = true;
        continue;
      }

      const modeSent = successfulSendFor(adapter, generation);
      if (modeSent.mode !== mode) {
        try {
          await dispatchActionForGameSession(
            { type: "SetPriorityPassingMode", data: { mode } },
            adapter,
            generation,
          );
        } catch {
          // As above, a rejected dispatch must remain retryable.
          if (!isCurrentSession(adapter, generation, hostPreferences)) syncRequested = true;
          continue;
        }

        if (
          isCurrentSession(adapter, generation, hostPreferences)
          && effectivePriorityPassingMode() === mode
        ) {
          lastSent = { ...successfulSendFor(adapter, generation), mode };
        } else {
          syncRequested = true;
        }
      }
    }
  } finally {
    syncInFlight = false;
    // A notification can land after the loop condition but before the flag is
    // cleared. Make sure that request is not stranded.
    if (syncRequested) void drainGameplayPreferenceSync();
  }
}

function sendGameplayPreferences(): void {
  syncRequested = true;
  void drainGameplayPreferenceSync();
}

/** Initialize local engines from device preferences; adopt host preferences on
 * attachments. Then push changes. Mount exactly once in `GameProvider`. */
export function useGameplayPreferencesSync(): void {
  useEffect(() => {
    const unsubGame = useGameStore.subscribe(
      (state) => [
        state.adapter,
        state.gameSessionGeneration,
        state.gameState !== null,
        state.engineCommitEpoch,
      ] as const,
      sendGameplayPreferences,
      { fireImmediately: true },
    );
    const unsubPreferences = usePreferencesStore.subscribe(sendGameplayPreferences);
    // Full Control lives in `uiStore` (a per-session toggle, not a persisted
    // preference), so it needs its own subscription to reach the engine.
    // `uiStore` is a plain zustand store with no `subscribeWithSelector`
    // middleware, so the selector overload is unavailable — hence the explicit
    // previous-value guard, which also keeps unrelated UI state changes from
    // re-dispatching the preference.
    let lastFullControl = useUiStore.getState().fullControl;
    const unsubFullControl = useUiStore.subscribe((state) => {
      if (state.fullControl === lastFullControl) return;
      lastFullControl = state.fullControl;
      sendGameplayPreferences();
    });

    return () => {
      unsubGame();
      unsubPreferences();
      unsubFullControl();
    };
  }, []);
}
