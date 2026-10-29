import type { AudioThemeManifest, GamePhaseTag } from "./types";

/**
 * R2 public base URL for music assets. In production, music is served from R2 CDN.
 * In development, falls back to local files in /audio/music/.
 * Declared as __AUDIO_BASE_URL__ so Vite can inject it at build time.
 */
declare const __AUDIO_BASE_URL__: string;
const AUDIO_BASE_URL =
  typeof __AUDIO_BASE_URL__ !== "undefined" ? __AUDIO_BASE_URL__ : "";

/** Battle-phase track helper. Uses relative URLs resolved against baseUrl. */
function track(id: string, name: string, phase: GamePhaseTag) {
  return {
    id,
    url: `planeswalker-${id}.m4a`,
    phase,
    label: name,
  };
}

/** Planeswalker theme — AI-generated orchestral soundtrack with phase-aware battlefield music. */
export const PLANESWALKER_THEME: AudioThemeManifest = {
  id: "planeswalker",
  name: "Planeswalker",
  version: 1,
  author: "phase.rs (AI-generated via Suno)",
  description:
    "Epic fantasy orchestral soundtrack with phase-aware battlefield music that intensifies as the game progresses",
  baseUrl: AUDIO_BASE_URL || `${import.meta.env.BASE_URL}audio/music`,
  phaseBreakpoints: { mid: 5, late: 10 },
  sfx: [
    { eventType: "DamageDealt", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_combat_block_001.m4a` },
    { eventType: "LifeGained", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_life_gain_001.m4a` },
    { eventType: "LifeLost", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_life_loss_001.m4a` },
    { eventType: "SpellCast", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_spell_cast_001.m4a` },
    { eventType: "CreatureDestroyed", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_creature_destroy_001.m4a` },
    { eventType: "AttackersDeclared", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_attack_declare_001.m4a` },
    { eventType: "BlockersDeclared", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_combat_block_001.m4a` },
    { eventType: "LandPlayed", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_land_play_001.m4a` },
    { eventType: "CardDrawn", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_card_draw_002.m4a` },
    { eventType: "SpellCountered", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_spell_counter_001.m4a` },
    { eventType: "TokenCreated", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_token_create_001.m4a` },
    { eventType: "GameStarted", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_game_start_001.m4a` },
    { eventType: "PermanentSacrificed", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_sacrifice_001.m4a` },
    { eventType: "CounterAdded", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_counter_add_001.m4a` },
    { eventType: "AbilityActivated", url: `${import.meta.env.BASE_URL}audio/sfx/sfx_ability_activate_001.m4a` },
  ],
  music: {
    menu: [
      track("menu-main", "Hall of the Planeswalkers", "any"),
      track("menu-alt", "Between the Planes", "any"),
    ],
    deck_builder: [
      track("deck-builder", "The Artificer's Workshop", "any"),
      track("deck-builder-alt", "Tome of Strategies", "any"),
    ],
    lobby: [track("lobby-waiting", "The Summoning Circle", "any")],
    battlefield: [
      track("battle-early-1", "Opening Gambit", "early"),
      track("battle-early-2", "First Draw", "early"),
      track("battle-mid-1", "Clash of Wills", "mid"),
      track("battle-mid-2", "Arcane Escalation", "mid"),
      track("battle-mid-3", "Creatures of the Aether", "mid"),
      track("battle-late-1", "Final Reckoning", "late"),
      track("battle-late-2", "The Last Stand", "late"),
      track("battle-any-1", "Duelist's Resolve", "any"),
    ],
    victory: [track("victory", "Triumphant Ascension", "any")],
    defeat: [track("defeat", "Ashes of Defeat", "any")],
  },
};
