//! Shared helpers for the ported TS tests.
#![allow(dead_code)]

use euchre_engine::{Card, Seat, Suit};

/// Parses a card id literal such as `"JD"`.
pub fn c(id: &str) -> Card {
    Card::parse(id).unwrap_or_else(|| panic!("bad card id {id:?}"))
}

/// Parses a list of card id literals.
pub fn cards(ids: &[&str]) -> Vec<Card> {
    ids.iter().map(|id| c(id)).collect()
}

/// Parses a suit letter such as `'H'`.
pub fn s(ch: char) -> Suit {
    Suit::from_char(ch).unwrap_or_else(|| panic!("bad suit {ch:?}"))
}

/// The seat numbered `n` (panics unless 0..=3).
pub fn seat(n: u8) -> Seat {
    Seat::new(n).unwrap_or_else(|| panic!("bad seat {n}"))
}

/// `[...xs].sort()` on card ids (lexicographic, like JS).
pub fn sorted(xs: &[Card]) -> Vec<Card> {
    let mut v = xs.to_vec();
    v.sort();
    v
}

/// The four suits in TS `ALL_SUITS` order.
pub const ALL_SUITS: [Suit; 4] = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs];

/* -------------------------------------------------------------------------- */
/* The reducer harness (ported from reduce/game/project.test.ts)               */
/* -------------------------------------------------------------------------- */

use euchre_engine::{
    ApplyError, ApplyResult, EngineConfigOverrides, GameInit, GameState, GameStatus, HandState,
    LegalMove, Move, RuleCode, RuleError, advance, apply, create_game, fnv1a, legal_moves,
    mulberry32, try_advance, try_apply,
};

/// The four seats in order.
pub const SEATS_ALL: [Seat; 4] = Seat::ALL;

/// A move chooser: `(state, seat, legal) -> chosen`.
pub type Picker = Box<dyn FnMut(&GameState, Seat, &[LegalMove]) -> LegalMove>;

/// `legal[floor(rnd() * legal.length)]`.
pub fn pick_index(r: f64, len: usize) -> usize {
    ((r * len as f64).floor() as usize).min(len.saturating_sub(1))
}

/// `randomPicker(seed)`: a uniform choice from `mulberry32(fnv1a(seed))`.
pub fn random_picker(seed: &str) -> Picker {
    let mut rnd = mulberry32(fnv1a(seed));
    Box::new(move |_s, _seat, legal| legal[pick_index(rnd(), legal.len())].clone())
}

/// `passHeavyPicker(seed)` (game.test.ts): passes 4 times in 5 while bidding.
pub fn pass_heavy_picker(seed: &str) -> Picker {
    let mut rnd = mulberry32(fnv1a(&format!("{seed}#pass-heavy")));
    Box::new(move |state, _seat, legal| {
        let bidding = matches!(
            state.hand.phase,
            euchre_engine::GamePhase::BidRound1 | euchre_engine::GamePhase::BidRound2
        );
        if bidding
            && rnd() < 0.8
            && let Some(pass) = legal.iter().find(|m| m.id == "pass")
        {
            return pass.clone();
        }
        legal[pick_index(rnd(), legal.len())].clone()
    })
}

/// `alonePicker(seed)` (project.test.ts): prefers any `+alone` bid.
pub fn alone_picker(seed: &str) -> Picker {
    let mut fallback = random_picker(seed);
    Box::new(
        move |state, seat, legal| match legal.iter().find(|m| m.id.ends_with("+alone")) {
            Some(m) => m.clone(),
            None => fallback(state, seat, legal),
        },
    )
}

/// `aloneAtPicker(seed, skip)` (project.test.ts): `skip` round-1 passes, then
/// `orderUp+alone`.
pub fn alone_at_picker(seed: &str, skip: u8) -> Picker {
    let mut fallback = random_picker(&format!("{seed}#{skip}"));
    Box::new(move |state, seat, legal| {
        if state.hand.phase == euchre_engine::GamePhase::BidRound1 {
            let want = if state.hand.passes < skip {
                "pass"
            } else {
                "orderUp+alone"
            };
            return legal
                .iter()
                .find(|m| m.id == want)
                .cloned()
                .unwrap_or_else(|| panic!("aloneAtPicker: {want} not offered"));
        }
        fallback(state, seat, legal)
    })
}

/// An `on_state` observer.
pub type StateHook<'a> = Box<dyn FnMut(&GameState) + 'a>;

/// An `on_transition` observer: `(before, result, seat)`.
pub type TransitionHook<'a> = Box<dyn FnMut(&GameState, &ApplyResult, Option<Seat>) + 'a>;

/// Observer hooks for [`run_game`] / [`run_until`].
#[derive(Default)]
pub struct RunHooks<'a> {
    /// Called with every state, the initial one included.
    pub on_state: Option<StateHook<'a>>,
    /// Called with `(before, result, seat)` after every transition.
    pub on_transition: Option<TransitionHook<'a>>,
    /// Transition budget.
    pub budget: Option<usize>,
}

impl<'a> RunHooks<'a> {
    /// Hooks with only `on_state`.
    pub fn on_state(f: impl FnMut(&GameState) + 'a) -> RunHooks<'a> {
        RunHooks {
            on_state: Some(Box::new(f)),
            ..RunHooks::default()
        }
    }

    /// Hooks with only `on_transition`.
    pub fn on_transition(
        f: impl FnMut(&GameState, &ApplyResult, Option<Seat>) + 'a,
    ) -> RunHooks<'a> {
        RunHooks {
            on_transition: Some(Box::new(f)),
            ..RunHooks::default()
        }
    }
}

fn step_once<P>(state: &GameState, pick: &mut P) -> Result<(ApplyResult, Option<Seat>), String>
where
    P: FnMut(&GameState, Seat, &[LegalMove]) -> LegalMove + ?Sized,
{
    let seat = state.hand.turn_seat;
    let res = match seat {
        None => try_advance(state).map_err(|e| e.to_string())?,
        Some(seat) => {
            let legal = legal_moves(state, seat);
            if legal.is_empty() {
                return Err(format!(
                    "V19: empty legal set for seat {seat} in {}",
                    state.hand.phase
                ));
            }
            let chosen = pick(state, seat, &legal);
            match try_apply(state, seat, chosen.mv, None) {
                Ok(r) => r,
                Err(ApplyError::Rule(e)) => return Err(format!("RuleError {e}")),
                Err(ApplyError::Fault(e)) => return Err(e.to_string()),
            }
        }
    };
    Ok((res, seat))
}

/// `runGame`, returning the TS throw as `Err`.
pub fn try_run_game<P>(
    initial: GameState,
    mut pick: P,
    mut hooks: RunHooks<'_>,
) -> Result<GameState, String>
where
    P: FnMut(&GameState, Seat, &[LegalMove]) -> LegalMove,
{
    let budget = hooks.budget.unwrap_or(20000);
    let mut state = initial;
    let mut n = 0;
    if let Some(f) = hooks.on_state.as_mut() {
        f(&state);
    }
    while state.status == GameStatus::Active {
        n += 1;
        if n > budget {
            return Err(format!("runGame: budget exhausted in {}", state.hand.phase));
        }
        let (res, seat) = step_once(&state, &mut pick)?;
        if res.state == state {
            return Err(format!("runGame: stalled in {}", state.hand.phase));
        }
        if let Some(f) = hooks.on_transition.as_mut() {
            f(&state, &res, seat);
        }
        state = res.state;
        if let Some(f) = hooks.on_state.as_mut() {
            f(&state);
        }
    }
    Ok(state)
}

/// Drives a match to completion: `advance` whenever no seat is on turn.
pub fn run_game<P>(initial: GameState, pick: P, hooks: RunHooks<'_>) -> GameState
where
    P: FnMut(&GameState, Seat, &[LegalMove]) -> LegalMove,
{
    try_run_game(initial, pick, hooks).unwrap_or_else(|e| panic!("{e}"))
}

/// Drives until `pred` holds, then stops (`runUntil`, budget 2000).
pub fn run_until<P>(
    initial: GameState,
    mut pick: P,
    pred: impl Fn(&GameState) -> bool,
    mut hooks: RunHooks<'_>,
) -> GameState
where
    P: FnMut(&GameState, Seat, &[LegalMove]) -> LegalMove,
{
    let budget = hooks.budget.unwrap_or(2000);
    let mut state = initial;
    let mut n = 0;
    if let Some(f) = hooks.on_state.as_mut() {
        f(&state);
    }
    while !pred(&state) {
        n += 1;
        assert!(
            n <= budget,
            "runUntil: budget exhausted in {}",
            state.hand.phase
        );
        assert!(
            state.status == GameStatus::Active,
            "runUntil: match ended before the predicate"
        );
        let (res, seat) = step_once(&state, &mut pick).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            res.state != state,
            "runUntil: stalled in {}",
            state.hand.phase
        );
        if let Some(f) = hooks.on_transition.as_mut() {
            f(&state, &res, seat);
        }
        state = res.state;
        if let Some(f) = hooks.on_state.as_mut() {
            f(&state);
        }
    }
    state
}

/// `createGame({ gameId: "g-" + seed, seed, cfg, firstDealer })`.
pub fn game(
    seed: &str,
    first_dealer: Option<Seat>,
    cfg: Option<EngineConfigOverrides>,
) -> GameState {
    create_game(GameInit {
        game_id: format!("g-{seed}"),
        seed: seed.to_owned(),
        cfg,
        internal_token: None,
        first_dealer,
    })
}

/// A match in `cutting`, ready for the cut.
pub fn opened(
    seed: &str,
    first_dealer: Option<Seat>,
    cfg: Option<EngineConfigOverrides>,
) -> GameState {
    advance(&game(seed, first_dealer, cfg)).state
}

/// The seat on turn (panics if none).
pub fn turn(s: &GameState) -> Seat {
    s.hand
        .turn_seat
        .unwrap_or_else(|| panic!("expected a seat to be on turn"))
}

/// Applies a move that must be legal.
pub fn ok(state: &GameState, seat: Seat, mv: Move) -> ApplyResult {
    apply(state, seat, mv, None).unwrap_or_else(|e| panic!("unexpected RuleError {e}"))
}

/// Answer the cut and land in `bid_round_1`.
pub fn after_cut(state: &GameState, cut: bool) -> GameState {
    ok(state, turn(state), Move::Cut { cut }).state
}

/// `n` passes by whoever is on turn.
pub fn passes(state: &GameState, n: usize) -> GameState {
    let mut s = state.clone();
    for _ in 0..n {
        s = ok(&s, turn(&s), Move::Pass).state;
    }
    s
}

/// A hand-patched state. Only used where a specific card layout is the test.
pub fn patch_hand(state: &GameState, change: impl FnOnce(&mut HandState)) -> GameState {
    let mut s = state.clone();
    change(&mut s.hand);
    s
}

/// Asserts `r` rejected with exactly `code`, and hands the error back.
pub fn expect_rule(code: RuleCode, r: Result<ApplyResult, RuleError>) -> RuleError {
    match r {
        Ok(_) => panic!("expected RuleError({code}) but the move was accepted"),
        Err(e) => {
            assert_eq!(e.code, code, "expected RuleError({code}), got {e}");
            e
        }
    }
}

/// Override builder: `{ gameTo }`.
pub fn cfg_game_to(game_to: u8) -> Option<EngineConfigOverrides> {
    Some(EngineConfigOverrides {
        game_to: Some(game_to),
        ..EngineConfigOverrides::default()
    })
}

/// Override builder: `{ stickTheDealer }`.
pub fn cfg_stick(stick: bool) -> Option<EngineConfigOverrides> {
    Some(EngineConfigOverrides {
        stick_the_dealer: Some(stick),
        ..EngineConfigOverrides::default()
    })
}

/// `JSON.stringify` of any serializable value.
pub fn json<T: serde::Serialize>(x: &T) -> String {
    serde_json::to_string(x).unwrap_or_else(|e| panic!("serialize: {e}"))
}
