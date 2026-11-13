//! Certified combo lines: find the line that wins with a catalogued combo, and
//! play it.
//!
//! ## The defect this closes
//!
//! The AI scores one action at a time, and no step of a combo scores well alone:
//! shooting its own creature, sacrificing one, untapping a permanent for no
//! visible reason. With every piece of a known combo on the battlefield and the
//! engine's loop detector on, an AI seat never played the first lap, so the engine
//! never had a loop to recognise (`tests/combo_execution.rs` records the
//! baseline: no wins in 32 games across eight assembled combos).
//!
//! ## Shape
//!
//! 1. The session knows which catalogued combos are in the seat's deck
//!    ([`super::catalog`]). A combo is *live* once every card it names is
//!    somewhere the seat can use it (any zone but the library and the stack).
//! 2. [`find_winning_line`] searches for a line on copies of the game. At
//!    priority it tries the named cards' casts and activations, mana abilities
//!    included (CR 605.3a), and once it holds an unbounded resource, the
//!    seat's other cards too; at a prompt it tries the engine-issued answers. Opponents are assumed to pass (CR 117.3d) and to accept a
//!    proposed shortcut (CR 732.2c). When the engine offers a loop shortcut
//!    (CR 732.2a) that wins or makes an unbounded resource, the search takes
//!    it; an offer only to repeat what the seat just did is a choice like any
//!    other. Nothing here knows what a card does.
//! 3. Only a line the reducer plays to a won game is kept (CR 104.2). Its
//!    steps are stored in the session, and at each of the seat's next decisions
//!    [`next_line_action`] returns the next step, as long as the game still
//!    stands where the line expected it to.
//!
//! A loop that only makes a resource (mana, tokens, counters) is one stage of a
//! line: the search takes a fixed number of repetitions and searches on from the
//! board that leaves, for the payoff that spends it. Eliminating one of several
//! opponents starts a new stage the same way.
//!
//! ## What it does not do
//!
//! - The search never passes priority on an empty stack, so a combo whose win is
//!   an attack (a token loop) is not found here.
//! - Until a line holds an unbounded resource it tries only the cards the
//!   catalog names. After that it tries every cast and activation the seat
//!   has, so a payoff the entry does not list is found if the budget reaches
//!   it; nothing ranks one unnamed card above another.
//! - It runs only on the seat's own main phase with an empty stack, and only
//!   when the game's loop detection is interactive, because the shortcut is how
//!   an unbounded loop ends.
//! - A search is bounded by [`MAX_WORK`] and, outside measurement mode, by the
//!   clock. One that runs out is not repeated for [`RETRY_AFTER_TURNS`] of the
//!   seat's turns, and a seat searches at most [`MAX_SEARCHES_PER_TURN`] times a
//!   turn.
//! - A preset with no play lookahead, or one that samples hidden zones, never
//!   searches, and neither does a seat whose turn another player controls.
//! - The loop guards in `search` (activations and casts per turn) do not apply
//!   to a line: the reducer has already played it to the end.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::mem::{discriminant, Discriminant};

use engine::ai_support::AiDecisionContract;
use engine::analysis::decision_template::ShortcutDecisionSchema;
use engine::analysis::loop_check::{LoopCertificate, ShortcutResponse};
use engine::analysis::{DecisionTemplate, IterationCount, ResourceAxis, WinKind};
use engine::game::engine::apply_as_current_for_simulation;
use engine::game::game_object::GameObject;
use engine::game::mana_sources::activatable_mana_actions_for_player;
use engine::types::ability::TargetRef;
use engine::types::actions::GameAction;
use engine::types::game_state::{GameState, LoopDetectionMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;
use engine::util::Deadline;

use crate::config::AiConfig;
use crate::session::AiSession;

use super::catalog::CatalogCombo;

/// The work one search may spend across every depth it tries: one unit for each
/// reducer apply, and more for a shortcut, which the engine plays out cycle by
/// cycle.
const MAX_WORK: usize = 4000;

/// Shortcut repetitions that cost one unit of work.
const REPETITIONS_PER_WORK: u32 = 8;

/// Permanents on the battlefield that add one unit of work to an apply: every
/// reducer step is slower on a wide board.
const PERMANENTS_PER_WORK: usize = 16;

/// Wall-clock budget of one search outside measurement mode.
const SEARCH_BUDGET_MS: u32 = 2000;

/// The depths tried in turn, in choices the seat makes per stage. Eight is the
/// shallowest that finds a two-lap loop with its prompts; a pass at less only
/// spends the budget.
const DEPTHS: [usize; 3] = [8, 12, 16];

/// How many times one line may start over at full depth: once for each unbounded
/// resource it takes and each opponent it eliminates.
const STAGES: usize = 6;

/// Answers tried at one of the seat's prompts.
const PROMPT_BRANCHES: usize = 5;

/// Repetitions taken from an offer that makes an unbounded resource, for each
/// point of life the opponents have between them: enough mana to pay an X spell
/// twice over that total.
const RESOURCE_REPETITIONS_PER_LIFE: u32 = 2;

/// Repetitions taken from an offer that makes a new permanent each time. A board
/// of hundreds of tokens is slow to play on and no use to a search that does not
/// attack.
const OBJECT_REPETITIONS: u32 = 20;

/// The seat's own turns before a position whose search ran out of budget is
/// searched again.
const RETRY_AFTER_TURNS: u32 = 3;

/// Searches one seat may start in one turn, however the position changes.
const MAX_SEARCHES_PER_TURN: u8 = 3;

/// Shortcut declarations in a row that cost the search no depth. A loop of
/// triggers can offer again at once; past this many each answer is a step.
const FREE_DECLARATIONS: usize = 3;

/// Times a line hands out the same step for the same state before it gives up:
/// a step the reducer refused would otherwise be offered for ever.
const MAX_REASKS: u8 = 2;

/// A coarse description of a decision point, compared between the copy a line
/// was found on and the game it is played in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    prompt: &'static str,
    turn: u32,
    phase: Phase,
    stack: usize,
    battlefield: usize,
    life: Vec<i32>,
    hand: usize,
    floating_mana: usize,
}

impl Fingerprint {
    fn of(state: &GameState, player: PlayerId) -> Self {
        let seat = state.players.iter().find(|seat| seat.id == player);
        Fingerprint {
            prompt: state.waiting_for.variant_name(),
            turn: state.turn_number,
            phase: state.phase,
            stack: state.stack.len(),
            battlefield: state.battlefield.len(),
            life: state.players.iter().map(|player| player.life).collect(),
            hand: seat.map_or(0, |seat| seat.hand.len()),
            floating_mana: seat.map_or(0, |seat| seat.mana_pool.total()),
        }
    }
}

/// One of the seat's actions in a certified line, and where it is taken.
#[derive(Debug, Clone)]
pub(crate) struct LineStep {
    action: GameAction,
    at: Fingerprint,
}

/// A certified line being played out, one step per decision.
#[derive(Debug, Clone)]
pub(crate) struct PlannedLine {
    steps: Vec<LineStep>,
    /// The index of the next step to hand out.
    next: usize,
    /// The revision of the state the last step was handed out for. A second
    /// request for the same state gets the same step.
    issued_at: Option<u64>,
    /// How often the last step has been handed out again for that state.
    reasks: u8,
}

impl PlannedLine {
    fn new(steps: Vec<LineStep>) -> Self {
        PlannedLine {
            steps,
            next: 0,
            issued_at: None,
            reasks: 0,
        }
    }

    /// The step for `state`, or `None` when the line is spent or the game no
    /// longer stands where the line expected.
    fn step_for(&mut self, state: &GameState, player: PlayerId) -> Option<GameAction> {
        if self.issued_at == Some(state.state_revision) {
            if self.reasks >= MAX_REASKS {
                return None;
            }
            self.reasks += 1;
            return self
                .next
                .checked_sub(1)
                .and_then(|last| self.steps.get(last))
                .map(|step| step.action.clone());
        }
        let step = self.steps.get(self.next)?;
        if step.at != Fingerprint::of(state, player) {
            return None;
        }
        self.next += 1;
        self.issued_at = Some(state.state_revision);
        self.reasks = 0;
        Some(step.action.clone())
    }
}

/// What a seat last searched, so a position that held no line is not searched
/// again at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SearchMemo {
    /// Where the live combos' cards were.
    pieces: u64,
    /// What the seat had to play them with: untapped permanents and cards in
    /// hand. A land played after a search is a new position.
    means: (usize, usize),
    turn: u32,
    phase: Phase,
    /// How many of the seat's own turns had begun.
    own_turns: u32,
    /// Searches started this turn, this one included.
    searches: u8,
    /// Whether the search tried every line it could reach. One that ran out of
    /// budget instead would only run out again, so it waits
    /// [`RETRY_AFTER_TURNS`].
    exhaustive: bool,
    /// Whether a line found here was dropped because the game left it. The
    /// same position may then be searched once more.
    dropped: bool,
}

impl SearchMemo {
    /// Whether this memo already answers a search of the given position.
    fn covers(&self, at: &Position) -> bool {
        if self.turn == at.turn && self.searches >= MAX_SEARCHES_PER_TURN {
            return true;
        }
        if self.pieces != at.pieces {
            return false;
        }
        if !self.exhaustive && at.own_turns < self.own_turns.saturating_add(RETRY_AFTER_TURNS) {
            return true;
        }
        !self.dropped && self.turn == at.turn && self.phase == at.phase && self.means == at.means
    }
}

/// The position a search would start from, as far as a memo distinguishes one.
struct Position {
    pieces: u64,
    means: (usize, usize),
    turn: u32,
    phase: Phase,
    own_turns: u32,
}

impl Position {
    fn of(state: &GameState, ai_player: PlayerId, pieces: u64) -> Self {
        let seat = state.players.iter().find(|seat| seat.id == ai_player);
        let untapped = state
            .battlefield
            .iter()
            .filter_map(|id| state.objects.get(id))
            .filter(|object| object.controller == ai_player && !object.tapped)
            .count();
        Position {
            pieces,
            means: (untapped, seat.map_or(0, |seat| seat.hand.len())),
            turn: state.turn_number,
            phase: state.phase,
            own_turns: seat.map_or(0, |seat| seat.turns_taken),
        }
    }
}

/// The certified-line answer to this decision: the next step of a line already
/// being played, or the first step of one found now. `None` leaves the decision
/// to the ordinary paths.
pub(crate) fn next_line_action(
    state: &GameState,
    ai_player: PlayerId,
    config: &AiConfig,
    session: &AiSession,
    contract: &AiDecisionContract,
) -> Option<GameAction> {
    // The search plays the real hidden zones, as every shipped preset's search
    // does; a preset that samples them instead is left to the ordinary paths.
    // CR 723.5: a player who controls another's turn decides for them. A line
    // that wins the game for the controlled player is not theirs to play.
    if !config.play_lookahead
        || config.search.determinization_samples != 0
        || contract.authorized_actor != contract.semantic_owner
    {
        return None;
    }
    let combos = session.combos.get(&ai_player)?;
    // A chooser may be asked on behalf of a seat that owes nothing here. That is
    // not the game leaving the line, so the line is kept.
    if combos.is_empty() || !state.waiting_for.acting_players().contains(&ai_player) {
        return None;
    }

    if let Ok(mut lines) = session.combo_lines.write() {
        if let Some(line) = lines.get_mut(&ai_player) {
            match line
                .step_for(state, ai_player)
                .filter(|action| contract.contains_action(state, action))
            {
                Some(action) => return Some(action),
                None => {
                    // The game left the line (an opponent acted, or the line is
                    // spent). Forget it, and let this position be searched again.
                    lines.remove(&ai_player);
                    if let Ok(mut searched) = session.combo_searched.write() {
                        if let Some(memo) = searched.get_mut(&ai_player) {
                            memo.dropped = true;
                        }
                    }
                }
            }
        }
    }

    if !search_may_start(state, ai_player) {
        return None;
    }
    let (pieces, signature) = live_pieces(state, ai_player, combos)?;
    let position = Position::of(state, ai_player, signature);
    let earlier = session.combo_searched.read().ok()?.get(&ai_player).copied();
    if earlier.is_some_and(|memo| memo.covers(&position)) {
        return None;
    }
    let deadline = if config.execution_mode.is_measurement() {
        Deadline::none()
    } else {
        match config.search.time_budget_ms {
            // A decision given no time to search is given none for this either.
            Some(0) => return None,
            Some(budget) => Deadline::after(budget.min(SEARCH_BUDGET_MS)),
            None => Deadline::after(SEARCH_BUDGET_MS),
        }
    };
    let outcome = find_winning_line(state, ai_player, &pieces, deadline);
    session.combo_searched.write().ok()?.insert(
        ai_player,
        SearchMemo {
            pieces: position.pieces,
            means: position.means,
            turn: position.turn,
            phase: position.phase,
            own_turns: position.own_turns,
            searches: earlier
                .filter(|memo| memo.turn == position.turn)
                .map_or(1, |memo| memo.searches.saturating_add(1)),
            exhaustive: !matches!(outcome, SearchOutcome::OutOfBudget),
            dropped: false,
        },
    );
    let SearchOutcome::Found(steps) = outcome else {
        return None;
    };
    let mut line = PlannedLine::new(steps);
    let first = line
        .step_for(state, ai_player)
        .filter(|action| contract.contains_action(state, action))?;
    session.combo_lines.write().ok()?.insert(ai_player, line);
    Some(first)
}

/// CR 117.1a: the search opens on the seat's own main phase with an empty
/// stack, where every card it might cast can be cast. CR 732.2a: it needs the
/// engine to offer shortcuts, which only the interactive mode does.
fn search_may_start(state: &GameState, ai_player: PlayerId) -> bool {
    state.loop_detection == LoopDetectionMode::Interactive
        && state.active_player == ai_player
        && state.stack.is_empty()
        && matches!(state.phase, Phase::PreCombatMain | Phase::PostCombatMain)
        && matches!(state.waiting_for, WaitingFor::Priority { player } if player == ai_player)
}

/// The card names of every live combo, and a signature of where those cards are.
/// `None` when no combo is live.
fn live_pieces(
    state: &GameState,
    ai_player: PlayerId,
    combos: &[CatalogCombo],
) -> Option<(Vec<String>, u64)> {
    // On the battlefield a card is the seat's to use while the seat controls it;
    // anywhere else, while the seat owns it.
    let mine = |object: &GameObject| match object.zone {
        Zone::Library | Zone::Stack => false,
        Zone::Battlefield => object.controller == ai_player,
        _ => object.owner == ai_player,
    };
    let usable = |name: &str| {
        state
            .objects
            .values()
            .any(|object| mine(object) && object.name.eq_ignore_ascii_case(name))
    };
    let mut pieces: Vec<String> = Vec::new();
    for combo in combos {
        if combo.cards.iter().all(|card| usable(card)) {
            for card in &combo.cards {
                if !pieces.iter().any(|piece| piece.eq_ignore_ascii_case(card)) {
                    pieces.push(card.clone());
                }
            }
        }
    }
    if pieces.is_empty() {
        return None;
    }
    // By name and zone, not by object: a token copy of a piece is not a new
    // position to search.
    let mut located: Vec<(String, Zone)> = state
        .objects
        .values()
        .filter(|object| {
            mine(object)
                && pieces
                    .iter()
                    .any(|piece| piece.eq_ignore_ascii_case(&object.name))
        })
        .map(|object| (object.name.to_lowercase(), object.zone))
        .collect();
    located.sort_unstable();
    located.dedup();
    let mut hasher = DefaultHasher::new();
    located.hash(&mut hasher);
    Some((pieces, hasher.finish()))
}

/// Search for a line that wins the game for `ai_player` from `state`, trying at
/// priority only the cards named in `pieces`. Every step of the returned line
/// was applied by the reducer, and the last one ended the game with `ai_player`
/// the winner.
pub(crate) fn find_winning_line(
    state: &GameState,
    ai_player: PlayerId,
    pieces: &[String],
    deadline: Deadline,
) -> SearchOutcome {
    let mut search = Search {
        player: ai_player,
        pieces,
        work: 0,
        deadline,
        seen: HashMap::new(),
        stage_depth: 0,
        cut_short: false,
    };
    for depth in DEPTHS {
        search.seen.clear();
        search.stage_depth = depth;
        search.cut_short = false;
        let mut path = Path::default();
        let found = search.descend(state.clone(), depth, STAGES, 0, &mut path);
        tracing::debug!(
            player = ?ai_player,
            depth,
            found,
            work = search.work,
            steps = path.steps.len(),
            "combo line search pass"
        );
        if found {
            return SearchOutcome::Found(path.steps);
        }
        if search.spent() {
            return SearchOutcome::OutOfBudget;
        }
        // A pass that tried every line without stopping at its depth has shown
        // there is none; a deeper pass would walk the same tree.
        if !search.cut_short {
            return SearchOutcome::NoLine;
        }
    }
    SearchOutcome::OutOfBudget
}

/// How a search ended.
pub(crate) enum SearchOutcome {
    /// A line the reducer played to a won game.
    Found(Vec<LineStep>),
    /// Every line the search can reach was tried and none wins.
    NoLine,
    /// The work, time or depth allowed ran out first.
    OutOfBudget,
}

/// The parts of an open loop-shortcut offer the search reads.
struct Offer<'a> {
    predicted_winner: Option<PlayerId>,
    certificate: &'a LoopCertificate,
    schema: &'a ShortcutDecisionSchema,
    declaration: Option<&'a DecisionTemplate>,
}

/// CR 732.2a: the shortcut offer the game is waiting on `player` to answer as
/// its proposer, if that is what it is waiting on. The one place this module
/// reads an offer: it mints none, and what it declares against one goes through
/// the reducer like any other declaration.
fn own_offer(state: &GameState, player: PlayerId) -> Option<Offer<'_>> {
    match &state.waiting_for {
        WaitingFor::LoopShortcut {
            proposer,
            predicted_winner,
            certificate,
            schema,
            declaration,
            ..
        } if *proposer == player => Some(Offer {
            predicted_winner: *predicted_winner,
            certificate,
            schema,
            declaration: declaration.as_ref(),
        }),
        _ => None,
    }
}

/// A shortcut declaration the seat can make at one of its offers.
struct Declaration {
    action: GameAction,
    /// The kinds of unbounded resource it takes; none for a win or a repeat.
    kinds: Vec<Discriminant<ResourceAxis>>,
    /// Whether the search takes it without weighing the alternative.
    by_rule: bool,
}

/// The line under construction and the resources it has taken.
#[derive(Default)]
struct Path {
    steps: Vec<LineStep>,
    /// The kinds of unbounded resource already taken on this line. A second
    /// offer of a kind the line already holds is declined: it has the mana, and
    /// more of it finds nothing new.
    taken: Vec<Discriminant<ResourceAxis>>,
}

impl Path {
    fn mark(&self) -> (usize, usize) {
        (self.steps.len(), self.taken.len())
    }

    fn rewind(&mut self, (steps, taken): (usize, usize)) {
        self.steps.truncate(steps);
        self.taken.truncate(taken);
    }
}

struct Search<'a> {
    player: PlayerId,
    pieces: &'a [String],
    work: usize,
    deadline: Deadline,
    /// Priority positions already searched, with the most depth each was
    /// searched to.
    seen: HashMap<u64, usize>,
    /// The depth a new stage starts with.
    stage_depth: usize,
    /// Whether this pass stopped at its depth limit with choices still open.
    cut_short: bool,
}

/// Where a position stands once everything that is not the seat's own choice
/// has been played.
enum Settled {
    Won,
    Dead,
    /// The seat has a choice to make. `new_stage` says the line took an unbounded
    /// resource or eliminated an opponent on the way here.
    Choice {
        state: Box<GameState>,
        new_stage: bool,
    },
}

impl Search<'_> {
    fn spent(&self) -> bool {
        self.work >= MAX_WORK || self.deadline.expired()
    }

    fn apply(&mut self, state: &mut GameState, action: GameAction) -> bool {
        self.work += 1 + state.battlefield.len() / PERMANENTS_PER_WORK;
        if let GameAction::DeclareShortcut {
            count: IterationCount::Fixed(count),
            ..
        } = &action
        {
            self.work += (count / REPETITIONS_PER_WORK) as usize;
        }
        apply_as_current_for_simulation(state, action).is_ok()
    }

    fn is_piece(&self, state: &GameState, id: ObjectId) -> bool {
        state.objects.get(&id).is_some_and(|object| {
            self.pieces
                .iter()
                .any(|piece| piece.eq_ignore_ascii_case(&object.name))
        })
    }

    fn opponents_alive(&self, state: &GameState) -> usize {
        state
            .players
            .iter()
            .filter(|seat| seat.id != self.player && !seat.is_eliminated)
            .count()
    }

    /// The life the living opponents have between them.
    fn opponents_life(&self, state: &GameState) -> u32 {
        state
            .players
            .iter()
            .filter(|seat| seat.id != self.player && !seat.is_eliminated)
            .map(|seat| u32::try_from(seat.life.max(0)).unwrap_or(0))
            .sum()
    }

    /// Record one of the seat's own actions, taken at `state`.
    fn record(&self, path: &mut Path, state: &GameState, action: &GameAction) {
        path.steps.push(LineStep {
            action: action.clone(),
            at: Fingerprint::of(state, self.player),
        });
    }

    /// The declaration the seat can make at its own shortcut offer. `None` when
    /// the offer is one to decline: a draw, somebody else's win, or more of a
    /// resource the line already holds.
    fn declaration(&self, state: &GameState, path: &Path) -> Option<Declaration> {
        let Offer {
            predicted_winner,
            certificate,
            schema,
            declaration,
        } = own_offer(state, self.player)?;
        // CR 732.2a: "until lethal" names no count, and the engine takes it only
        // against an offer that measured no bound and names the seat the winner.
        if predicted_winner == Some(self.player) && !schema.is_bounded() {
            return Some(Declaration {
                action: GameAction::DeclareShortcut {
                    count: IterationCount::UntilLethal,
                    template: None,
                },
                kinds: Vec::new(),
                by_rule: true,
            });
        }
        let resource = certificate.win_kind == Some(WinKind::Advantage);
        let (count, kinds) = if resource {
            let mut kinds: Vec<Discriminant<ResourceAxis>> =
                certificate.unbounded.iter().map(discriminant).collect();
            kinds.dedup();
            if kinds.iter().all(|kind| path.taken.contains(kind)) {
                return None;
            }
            let makes_objects = certificate
                .unbounded
                .iter()
                .any(|axis| matches!(axis, ResourceAxis::TokensCreated));
            let repetitions = if makes_objects {
                OBJECT_REPETITIONS
            } else {
                self.opponents_life(state)
                    .saturating_mul(RESOURCE_REPETITIONS_PER_LIFE)
            };
            (repetitions.min(schema.deliverable_capacity), kinds)
        } else {
            match schema.iteration_count {
                IterationCount::Fixed(count) => (count, Vec::new()),
                // A draw, or a win that is somebody else's.
                IterationCount::UntilLethal | IterationCount::WithoutEnd => return None,
            }
        };
        (count > 0).then(|| Declaration {
            action: GameAction::DeclareShortcut {
                count: IterationCount::Fixed(count),
                // The engine's own suggested declaration: no pin is built here.
                template: schema.suggestion(declaration).cloned(),
            },
            kinds,
            // A count that ends the game, and a resource, are always worth
            // taking. A plain repeat of what the seat just did is not: the next
            // lap may need it aimed somewhere else.
            by_rule: resource || certificate.win_kind.is_some(),
        })
    }

    /// Play everything that is not the seat's own choice: the opponents pass
    /// and accept, the seat's stack resolves, and the seat's shortcut offers are
    /// answered by rule.
    fn settle(&mut self, mut state: GameState, path: &mut Path) -> Settled {
        let opponents = self.opponents_alive(&state);
        let mut new_stage = false;
        // A resource the line has declared for, and the position a refused take
        // would leave. The engine can accept a declaration and still deliver no
        // repetition, so the resource counts as taken only once the board shows
        // it.
        let mut awaited: Option<(Vec<Discriminant<ResourceAxis>>, u64)> = None;
        loop {
            // CR 104.2: only a game the seat won certifies a line. Read before the
            // budget, so the apply that won the game is not thrown away.
            if let WaitingFor::GameOver { winner } = &state.waiting_for {
                return if *winner == Some(self.player) {
                    Settled::Won
                } else {
                    Settled::Dead
                };
            }
            if self.spent() {
                return Settled::Dead;
            }
            let action = match &state.waiting_for {
                _ if own_offer(&state, self.player).is_some() => {
                    let offer = self.declaration(&state, path);
                    if offer.as_ref().is_some_and(|offer| !offer.by_rule) {
                        // To repeat or not is the seat's choice.
                        break;
                    }
                    if let Some(Declaration {
                        action: declare,
                        kinds,
                        ..
                    }) = offer
                    {
                        // CR 732.2b: a declaration the engine takes opens the other
                        // players' window or ends the game. One it refuses hands
                        // priority back and still returns `Ok`, so the result is
                        // read, not assumed.
                        let mut taken = state.clone();
                        if self.apply(&mut taken, declare.clone())
                            && matches!(
                                taken.waiting_for,
                                WaitingFor::RespondToShortcut { .. } | WaitingFor::GameOver { .. }
                            )
                        {
                            if !kinds.is_empty() {
                                // A second offer arises only from a board the first
                                // take changed.
                                if let Some((earlier, _)) = awaited.take() {
                                    path.taken.extend(earlier);
                                    new_stage = true;
                                }
                                let mut declined = state.clone();
                                if self.apply(&mut declined, GameAction::DeclineShortcut) {
                                    awaited = Some((kinds, position_key(&declined)));
                                }
                            }
                            self.record(path, &state, &declare);
                            state = taken;
                            continue;
                        }
                    }
                    self.record(path, &state, &GameAction::DeclineShortcut);
                    GameAction::DeclineShortcut
                }
                WaitingFor::Priority { player } if *player == self.player => {
                    if state.stack.is_empty() {
                        break;
                    }
                    // CR 117.4: the seat's own object resolves once everyone passes.
                    self.record(path, &state, &GameAction::PassPriority);
                    GameAction::PassPriority
                }
                // CR 117.3d: the line assumes each opponent passes.
                WaitingFor::Priority { .. } => GameAction::PassPriority,
                // CR 732.2c: and accepts the shortcut the seat proposed.
                WaitingFor::RespondToShortcut { player, .. } if *player != self.player => {
                    GameAction::RespondToShortcut {
                        response: ShortcutResponse::Accept,
                    }
                }
                other => {
                    // The simulation applies an answer as the first player the
                    // prompt waits on, so a prompt is the seat's to answer only
                    // when the seat is the only one it waits on.
                    if other.acting_players() == [self.player] {
                        break;
                    }
                    // A choice the line cannot make: an opponent's, or one several
                    // players make at once.
                    return Settled::Dead;
                }
            };
            if !self.apply(&mut state, action) {
                return Settled::Dead;
            }
        }
        if let Some((kinds, refused)) = awaited {
            if position_key(&state) != refused {
                path.taken.extend(kinds);
                new_stage = true;
            }
        }
        new_stage |= self.opponents_alive(&state) < opponents;
        Settled::Choice {
            state: Box::new(state),
            new_stage,
        }
    }

    /// The seat's options at `state`, most promising first.
    fn options(&self, state: &GameState, path: &Path) -> Vec<GameAction> {
        if own_offer(state, self.player).is_some() {
            // CR 732.2a: suggesting the shortcut is optional.
            return self
                .declaration(state, path)
                .map(|offer| offer.action)
                .into_iter()
                .chain([GameAction::DeclineShortcut])
                .collect();
        }
        let holds_resource = !path.taken.is_empty();
        let at_priority = matches!(state.waiting_for, WaitingFor::Priority { .. });
        let mut actions: Vec<GameAction> = AiDecisionContract::issue(state, self.player)
            .candidates
            .into_iter()
            .map(|candidate| candidate.action)
            .collect();
        if at_priority {
            // CR 605.3a: a mana ability may be activated whenever its controller
            // has priority. The candidate sample leaves a permanent's out; a mana
            // loop is made of them.
            for action in activatable_mana_actions_for_player(state, self.player) {
                if matches!(action, GameAction::ActivateAbility { .. })
                    && !actions.contains(&action)
                {
                    actions.push(action);
                }
            }
        }
        let opponent = |player: &PlayerId| {
            *player != self.player
                && state
                    .players
                    .iter()
                    .any(|seat| seat.id == *player && !seat.is_eliminated)
        };
        let mut ranked: Vec<((u8, u32), GameAction)> = actions
            .into_iter()
            .filter_map(|action| {
                let rank = match &action {
                    GameAction::PassPriority => return None,
                    // Before the line holds a resource only the named cards are
                    // tried: they are what makes the loop. Once it holds one, any
                    // card may be what spends it, so every cast and activation
                    // the seat has is tried, the named cards first.
                    GameAction::ActivateAbility { source_id, .. } if at_priority => {
                        match (self.is_piece(state, *source_id), holds_resource) {
                            (true, _) => 1,
                            (false, true) => 4,
                            (false, false) => return None,
                        }
                    }
                    GameAction::CastSpell { object_id, .. } if at_priority => {
                        // With a resource in hand the payoff comes first; before
                        // that, the loop does.
                        match (self.is_piece(state, *object_id), holds_resource) {
                            (true, true) => 0,
                            (true, false) => 2,
                            (false, true) => 3,
                            (false, false) => return None,
                        }
                    }
                    _ if at_priority => return None,
                    // CR 601.2c: an opponent's face first, then a named card.
                    GameAction::ChooseTarget {
                        target: Some(TargetRef::Player(player)),
                    } => {
                        if opponent(player) {
                            0
                        } else {
                            3
                        }
                    }
                    GameAction::ChooseTarget {
                        target: Some(TargetRef::Object(id)),
                    } => {
                        if self.is_piece(state, *id) {
                            1
                        } else {
                            3
                        }
                    }
                    GameAction::SelectCards { cards }
                        if cards.iter().any(|id| self.is_piece(state, *id)) =>
                    {
                        1
                    }
                    GameAction::DecideOptionalEffect { accept: true } => 0,
                    // CR 107.3a: the largest X the seat can pay.
                    GameAction::ChooseX { value } => {
                        return Some(((0, u32::MAX - *value), action));
                    }
                    _ => 2,
                };
                Some(((rank, 0), action))
            })
            .collect();
        ranked.sort_by_key(|(rank, _)| *rank);
        if !at_priority {
            ranked.truncate(PROMPT_BRANCHES);
        }
        ranked.into_iter().map(|(_, action)| action).collect()
    }

    fn descend(
        &mut self,
        state: GameState,
        depth: usize,
        stages_left: usize,
        free_declarations: usize,
        path: &mut Path,
    ) -> bool {
        let mark = path.mark();
        let (state, new_stage) = match self.settle(state, path) {
            Settled::Won => return true,
            Settled::Dead => {
                path.rewind(mark);
                return false;
            }
            Settled::Choice { state, new_stage } => (*state, new_stage),
        };
        // Finding a loop and spending what it makes are two searches: the second
        // starts from the board the first one reached, with its own depth. So is
        // the search for the next opponent once one is out.
        let (depth, stages_left) = if new_stage && stages_left > 0 {
            (self.stage_depth, stages_left - 1)
        } else {
            (depth, stages_left)
        };
        // Taking the seat's own offer is not a step deeper: the offer is the
        // engine's, and what follows it is the next choice. Declining one is a
        // step, and so is a declaration past [`FREE_DECLARATIONS`] in a row.
        let at_offer =
            own_offer(&state, self.player).is_some() && free_declarations < FREE_DECLARATIONS;
        if depth == 0 && !at_offer {
            self.cut_short = true;
            path.rewind(mark);
            return false;
        }
        // Prompts are not keyed: each is reached from one keyed priority position.
        if matches!(state.waiting_for, WaitingFor::Priority { .. }) {
            let key = {
                let mut hasher = DefaultHasher::new();
                position_key(&state).hash(&mut hasher);
                stages_left.hash(&mut hasher);
                path.taken.len().hash(&mut hasher);
                hasher.finish()
            };
            if self
                .seen
                .get(&key)
                .is_some_and(|searched| *searched >= depth)
            {
                path.rewind(mark);
                return false;
            }
            self.seen.insert(key, depth);
        }
        for action in self.options(&state, path) {
            if self.spent() {
                break;
            }
            let step = path.mark();
            self.record(path, &state, &action);
            let mut next = state.clone();
            let free = at_offer && matches!(action, GameAction::DeclareShortcut { .. });
            let (below, declarations) = if free {
                (depth, free_declarations + 1)
            } else if depth == 0 {
                // Only the free declaration is open at the depth limit.
                self.cut_short = true;
                path.rewind(step);
                continue;
            } else {
                (depth - 1, 0)
            };
            if self.apply(&mut next, action)
                && self.descend(next, below, stages_left, declarations, path)
            {
                return true;
            }
            path.rewind(step);
        }
        path.rewind(mark);
        false
    }
}

/// A key for a priority position, for pruning lines that reach a position
/// already searched. It adds to [`crate::planner::quick_state_hash`] what a lap
/// of a loop changes and that hash leaves out: tapped state, counters, and how
/// often each ability has been activated this turn. The activation counts keep
/// a lap that returns to the same board from being pruned as a repeat, which is
/// the moment the engine offers its shortcut.
fn position_key(state: &GameState) -> u64 {
    let mut hasher = DefaultHasher::new();
    crate::planner::quick_state_hash(state).hash(&mut hasher);
    for id in &state.battlefield {
        let Some(object) = state.objects.get(id) else {
            continue;
        };
        object.tapped.hash(&mut hasher);
        // Order-independent: a counter map has no stable iteration order.
        let counters = object
            .counters
            .iter()
            .map(|(kind, count)| {
                let mut one = DefaultHasher::new();
                kind.hash(&mut one);
                count.hash(&mut one);
                one.finish()
            })
            .fold(0u64, u64::wrapping_add);
        counters.hash(&mut hasher);
    }
    let activations = state
        .activated_abilities_this_turn
        .iter()
        .map(|((id, index), count)| {
            let mut one = DefaultHasher::new();
            id.hash(&mut one);
            index.hash(&mut one);
            count.hash(&mut one);
            one.finish()
        })
        .fold(0u64, u64::wrapping_add);
    activations.hash(&mut hasher);
    hasher.finish()
}
