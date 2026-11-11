//! The shared type block for the euchre rules engine (port of `types.ts`).
//!
//! Types and constants only, plus the serde shapes that keep JSON byte-compatible
//! with the TS engine: camelCase keys, tagged unions on `"t"`, explicit `null` for
//! every `T | null` field, absent keys for every TS `?` field, and the hands record
//! as a `{"0": [...], "1": [...], "2": [...], "3": [...]}` object.
//!
//! `RuleError` lives with the reducer (`reduce.rs`), as in TS.

use std::fmt;
use std::str::FromStr;

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/* -------------------------------------------------------------------------- */
/* String-valued enums                                                         */
/* -------------------------------------------------------------------------- */

/// Error returned when a wire string does not name a variant of a string enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseEnumError {
    /// The Rust type that was being parsed.
    pub type_name: &'static str,
    /// The rejected input.
    pub value: String,
}

impl fmt::Display for ParseEnumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {}: {:?}", self.type_name, self.value)
    }
}

impl std::error::Error for ParseEnumError {}

/// Declares a fieldless enum whose serde form, `as_str`, `Display` and `FromStr`
/// all use one exact wire string per variant.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $s:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $s)] $variant ),+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The exact wire string of this variant.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $s),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ParseEnumError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($s => Ok($name::$variant),)+
                    _ => Err(ParseEnumError { type_name: stringify!($name), value: s.to_owned() }),
                }
            }
        }
    };
}

/* -------------------------------------------------------------------------- */
/* Cards                                                                       */
/* -------------------------------------------------------------------------- */

string_enum! {
    /// The four suits. `S`/`C` are the black pair, `H`/`D` the red pair.
    ///
    /// No `Ord`: the canonical order is `SUITS` (`S H D C`), which is not the
    /// lexicographic order of the wire strings.
    Suit {
        /// `"S"`.
        Spades => "S",
        /// `"H"`.
        Hearts => "H",
        /// `"D"`.
        Diamonds => "D",
        /// `"C"`.
        Clubs => "C",
    }
}

impl Suit {
    /// The single wire character, e.g. `'S'`.
    pub const fn as_char(self) -> char {
        match self {
            Suit::Spades => 'S',
            Suit::Hearts => 'H',
            Suit::Diamonds => 'D',
            Suit::Clubs => 'C',
        }
    }

    /// Parses a single wire character.
    pub const fn from_char(c: char) -> Option<Suit> {
        match c {
            'S' => Some(Suit::Spades),
            'H' => Some(Suit::Hearts),
            'D' => Some(Suit::Diamonds),
            'C' => Some(Suit::Clubs),
            _ => None,
        }
    }

    /// Position in the canonical `SUITS` order (`S 0, H 1, D 2, C 3`).
    pub const fn index(self) -> usize {
        self as usize
    }
}

string_enum! {
    /// The six euchre ranks. There is no 2-8 and no joker: the deck is 24 cards.
    ///
    /// No `Ord`: use `plain_rank` / `trump_rank`, which are the only orders that mean anything.
    Rank {
        /// `"9"`.
        Nine => "9",
        /// `"T"`.
        Ten => "T",
        /// `"J"`.
        Jack => "J",
        /// `"Q"`.
        Queen => "Q",
        /// `"K"`.
        King => "K",
        /// `"A"`.
        Ace => "A",
    }
}

impl Rank {
    /// The single wire character, e.g. `'T'`.
    pub const fn as_char(self) -> char {
        match self {
            Rank::Nine => '9',
            Rank::Ten => 'T',
            Rank::Jack => 'J',
            Rank::Queen => 'Q',
            Rank::King => 'K',
            Rank::Ace => 'A',
        }
    }

    /// Parses a single wire character.
    pub const fn from_char(c: char) -> Option<Rank> {
        match c {
            '9' => Some(Rank::Nine),
            'T' => Some(Rank::Ten),
            'J' => Some(Rank::Jack),
            'Q' => Some(Rank::Queen),
            'K' => Some(Rank::King),
            'A' => Some(Rank::Ace),
            _ => None,
        }
    }

    /// Position in the canonical `RANKS` order (`9 0 .. A 5`).
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// A card's stable identity, serialized as the 2-char `CardId` string
/// `"<Rank><Suit>"`, e.g. `"JS"`, `"TC"`, `"9D"`, `"AH"`.
///
/// `Ord` is the lexicographic order of that string, so `sort()` matches a JS
/// `CardId[].sort()` exactly. It is not a strength order.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Card {
    rank: Rank,
    suit: Suit,
}

/// The TS name for a card identity; the same type as [`Card`].
pub type CardId = Card;

impl Card {
    /// Builds a card from its rank and printed suit.
    pub const fn new(rank: Rank, suit: Suit) -> Card {
        Card { rank, suit }
    }

    /// The rank half of the card id.
    pub const fn rank(self) -> Rank {
        self.rank
    }

    /// The **printed** suit. Compare it only inside `cards.rs`; everywhere else use `effective_suit`.
    pub const fn printed_suit(self) -> Suit {
        self.suit
    }

    /// Parses a 2-char card id such as `"JD"`.
    pub fn parse(s: &str) -> Option<Card> {
        let mut chars = s.chars();
        let rank = Rank::from_char(chars.next()?)?;
        let suit = Suit::from_char(chars.next()?)?;
        if chars.next().is_some() {
            return None;
        }
        Some(Card { rank, suit })
    }

    /// The two wire bytes, e.g. `*b"JD"`.
    pub const fn id_bytes(self) -> [u8; 2] {
        [self.rank.as_char() as u8, self.suit.as_char() as u8]
    }
}

impl PartialOrd for Card {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Card {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id_bytes().cmp(&other.id_bytes())
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.rank.as_char(), self.suit.as_char())
    }
}

impl fmt::Debug for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Error returned when a string is not a valid 2-char card id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseCardError(pub String);

impl fmt::Display for ParseCardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid card id: {:?}", self.0)
    }
}

impl std::error::Error for ParseCardError {}

impl FromStr for Card {
    type Err = ParseCardError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Card::parse(s).ok_or_else(|| ParseCardError(s.to_owned()))
    }
}

impl Serialize for Card {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let bytes = self.id_bytes();
        // Both bytes are ASCII by construction.
        let s = std::str::from_utf8(&bytes).map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(s)
    }
}

impl<'de> Deserialize<'de> for Card {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CardVisitor;

        impl Visitor<'_> for CardVisitor {
            type Value = Card;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a 2-char card id such as \"JD\"")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Card, E> {
                Card::parse(v).ok_or_else(|| E::invalid_value(de::Unexpected::Str(v), &self))
            }
        }

        deserializer.deserialize_str(CardVisitor)
    }
}

/* -------------------------------------------------------------------------- */
/* Seats and teams                                                             */
/* -------------------------------------------------------------------------- */

/// Error returned when a number is out of range for a small integer type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutOfRange {
    /// The Rust type that was being built.
    pub type_name: &'static str,
    /// The rejected value.
    pub value: u8,
}

impl fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} out of range: {}", self.type_name, self.value)
    }
}

impl std::error::Error for OutOfRange {}

/// A chair at the table, clockwise: `0` South (the human), `1` West, `2` North
/// (the human's partner), `3` East. Serialized as a JSON number `0..=3`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Seat(u8);

impl Seat {
    /// Seat 0, South: the human.
    pub const SOUTH: Seat = Seat(0);
    /// Seat 1, West.
    pub const WEST: Seat = Seat(1);
    /// Seat 2, North: the human's partner.
    pub const NORTH: Seat = Seat(2);
    /// Seat 3, East.
    pub const EAST: Seat = Seat(3);
    /// Every seat in clockwise order.
    pub const ALL: [Seat; 4] = [Seat(0), Seat(1), Seat(2), Seat(3)];

    /// The seat numbered `n`, or `None` unless `n` is `0..=3`.
    pub const fn new(n: u8) -> Option<Seat> {
        if n < 4 { Some(Seat(n)) } else { None }
    }

    /// The seat numbered `n & 3` (JS `& 3` seat arithmetic).
    pub const fn wrapping(n: u8) -> Seat {
        Seat(n & 3)
    }

    /// The seat number `0..=3`.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The seat number as an array index.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl TryFrom<u8> for Seat {
    type Error = OutOfRange;

    fn try_from(n: u8) -> Result<Self, Self::Error> {
        Seat::new(n).ok_or(OutOfRange {
            type_name: "Seat",
            value: n,
        })
    }
}

impl From<Seat> for u8 {
    fn from(s: Seat) -> u8 {
        s.0
    }
}

impl fmt::Display for Seat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Seat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Seat({})", self.0)
    }
}

/// A partnership: team `0` is seats `{0, 2}` (always the human's team), team `1`
/// is `{1, 3}`. Serialized as a JSON number `0 | 1`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Team(u8);

impl Team {
    /// Team 0: South and North, the human's team.
    pub const NS: Team = Team(0);
    /// Team 1: West and East.
    pub const EW: Team = Team(1);
    /// Both teams, in index order.
    pub const ALL: [Team; 2] = [Team(0), Team(1)];

    /// The team numbered `n`, or `None` unless `n` is `0 | 1`.
    pub const fn new(n: u8) -> Option<Team> {
        if n < 2 { Some(Team(n)) } else { None }
    }

    /// The team number `0 | 1`.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The team number as an array index into `score` / `tricksWon`.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl TryFrom<u8> for Team {
    type Error = OutOfRange;

    fn try_from(n: u8) -> Result<Self, Self::Error> {
        Team::new(n).ok_or(OutOfRange {
            type_name: "Team",
            value: n,
        })
    }
}

impl From<Team> for u8 {
    fn from(t: Team) -> u8 {
        t.0
    }
}

impl fmt::Display for Team {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Team {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Team({})", self.0)
    }
}

/// A seat's role relative to the current dealer. Serialized as a JSON number `0..=3`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
#[repr(u8)]
pub enum SeatRole {
    /// `0`: the dealer.
    Dealer = 0,
    /// `1`: eldest, left of the dealer; bids and leads first.
    Eldest = 1,
    /// `2`: the dealer's partner.
    DealerPartner = 2,
    /// `3`: third seat, right of the dealer; the cutter.
    Third = 3,
}

impl SeatRole {
    /// The role number `0..=3`.
    pub const fn get(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for SeatRole {
    type Error = OutOfRange;

    fn try_from(n: u8) -> Result<Self, Self::Error> {
        match n {
            0 => Ok(SeatRole::Dealer),
            1 => Ok(SeatRole::Eldest),
            2 => Ok(SeatRole::DealerPartner),
            3 => Ok(SeatRole::Third),
            _ => Err(OutOfRange {
                type_name: "SeatRole",
                value: n,
            }),
        }
    }
}

impl From<SeatRole> for u8 {
    fn from(r: SeatRole) -> u8 {
        r as u8
    }
}

/* -------------------------------------------------------------------------- */
/* Phases and statuses                                                         */
/* -------------------------------------------------------------------------- */

string_enum! {
    /// Every node of the phase state machine. `deal`, `trick_resolve`,
    /// `hand_score` and `game_over` (and `lobby`) are engine-only phases.
    ///
    /// Each variant has an explicit rename: `rename_all = "snake_case"` would
    /// produce `bid_round1`, not `bid_round_1`.
    GamePhase {
        /// `"lobby"`: no hand open yet.
        Lobby => "lobby",
        /// `"cutting"`: the cutter decides bump or run 'em.
        Cutting => "cutting",
        /// `"deal"`: defensive resting point; the deal is atomic with the cut.
        Deal => "deal",
        /// `"bid_round_1"`: order up or pass the up-card.
        BidRound1 => "bid_round_1",
        /// `"dealer_discard"`: the dealer discards one of six.
        DealerDiscard => "dealer_discard",
        /// `"bid_round_2"`: name a suit other than the turned-down one.
        BidRound2 => "bid_round_2",
        /// `"trick_play"`: a seat is to play a card.
        TrickPlay => "trick_play",
        /// `"trick_resolve"`: a completed trick is held on the table.
        TrickResolve => "trick_resolve",
        /// `"hand_score"`: the hand has been scored.
        HandScore => "hand_score",
        /// `"game_over"`: terminal.
        GameOver => "game_over",
    }
}

string_enum! {
    /// Match-level status. `legal_moves` returns `[]` unless this is `active`.
    GameStatus {
        /// `"active"`.
        Active => "active",
        /// `"complete"`.
        Complete => "complete",
        /// `"abandoned"`.
        Abandoned => "abandoned",
    }
}

string_enum! {
    /// The rows of the scoring table. `throw_in` scores nothing.
    HandResult {
        /// `"point"`: makers took 3 or 4.
        Point => "point",
        /// `"march"`: makers took all 5.
        March => "march",
        /// `"lone_point"`: a lone maker took 3 or 4.
        LonePoint => "lone_point",
        /// `"lone_march"`: a lone maker took all 5.
        LoneMarch => "lone_march",
        /// `"euchre"`: makers took 0-2.
        Euchre => "euchre",
        /// `"throw_in"`: four passes in round 2 with stick-the-dealer off.
        ThrowIn => "throw_in",
    }
}

string_enum! {
    /// The closed set of machine codes `apply` may reject with.
    RuleCode {
        /// `"not_your_turn"`.
        NotYourTurn => "not_your_turn",
        /// `"wrong_phase"`.
        WrongPhase => "wrong_phase",
        /// `"illegal_move"`.
        IllegalMove => "illegal_move",
        /// `"must_follow_suit"`.
        MustFollowSuit => "must_follow_suit",
        /// `"card_not_in_hand"`.
        CardNotInHand => "card_not_in_hand",
        /// `"bidding_closed"`.
        BiddingClosed => "bidding_closed",
        /// `"dealer_must_call"`: a pass from the stuck dealer (never `bidding_closed`).
        DealerMustCall => "dealer_must_call",
        /// `"sitting_out"`: the loner's partner tried to act.
        SittingOut => "sitting_out",
        /// `"stale_turn"`: the caller's turn nonce is out of date.
        StaleTurn => "stale_turn",
    }
}

string_enum! {
    /// How a move reached `apply`. `auto` covers timeout and abandon auto-play.
    ActionKind {
        /// `"human"`.
        Human => "human",
        /// `"ai"`.
        Ai => "ai",
        /// `"auto"`.
        Auto => "auto",
    }
}

string_enum! {
    /// Where an AI seat's decision came from.
    AiDecisionSource {
        /// `"llm"`.
        Llm => "llm",
        /// `"fallback"`.
        Fallback => "fallback",
        /// `"forced"`.
        Forced => "forced",
    }
}

/// The TS name for [`AiDecisionSource`].
pub type AIDecisionSource = AiDecisionSource;

string_enum! {
    /// Why an `auto` action was synthesised by the server.
    AutoReason {
        /// `"ai_timeout"`.
        AiTimeout => "ai_timeout",
        /// `"ai_unreachable"`.
        AiUnreachable => "ai_unreachable",
        /// `"abandon"`.
        Abandon => "abandon",
    }
}

/* -------------------------------------------------------------------------- */
/* Moves                                                                       */
/* -------------------------------------------------------------------------- */

/// A bidding action, tagged on `"t"`. `alone` rides on the winning bid.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Bid {
    /// `{"t":"pass"}`.
    #[serde(rename = "pass")]
    Pass,
    /// `{"t":"orderUp","alone":bool}`: accept the up-card (round 1).
    #[serde(rename = "orderUp")]
    OrderUp {
        /// Whether the maker goes alone.
        alone: bool,
    },
    /// `{"t":"call","suit":Suit,"alone":bool}`: name trump (round 2).
    #[serde(rename = "call")]
    Call {
        /// The suit named.
        suit: Suit,
        /// Whether the maker goes alone.
        alone: bool,
    },
}

/// The full seat-action union accepted by `apply`, tagged on `"t"`. It has no
/// seat field by construction: the caller states who is acting.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Move {
    /// `{"t":"pass"}`.
    #[serde(rename = "pass")]
    Pass,
    /// `{"t":"orderUp","alone":bool}`.
    #[serde(rename = "orderUp")]
    OrderUp {
        /// Whether the maker goes alone.
        alone: bool,
    },
    /// `{"t":"call","suit":Suit,"alone":bool}`.
    #[serde(rename = "call")]
    Call {
        /// The suit named.
        suit: Suit,
        /// Whether the maker goes alone.
        alone: bool,
    },
    /// `{"t":"cut","cut":bool}`: `true` = bump, `false` = run 'em.
    #[serde(rename = "cut")]
    Cut {
        /// Whether the cutter bumps the deck.
        cut: bool,
    },
    /// `{"t":"discard","card":CardId}`: the dealer's face-down discard.
    #[serde(rename = "discard")]
    Discard {
        /// The card discarded.
        card: Card,
    },
    /// `{"t":"play","card":CardId}`: lay a card to the trick.
    #[serde(rename = "play")]
    Play {
        /// The card played.
        card: Card,
    },
}

/// The stable string id of a move: `pass`, `orderUp`, `orderUp+alone`, `call:H`,
/// `call:H+alone`, `cut:yes`, `cut:no`, `discard:9C`, `play:JS`.
pub type LegalMoveId = String;

impl Bid {
    /// The stable wire id of this bid (same as `Move::from(bid).id()`).
    pub fn id(self) -> LegalMoveId {
        Move::from(self).id()
    }

    /// Whether this bid declares going alone (`false` for a pass).
    pub const fn is_alone(self) -> bool {
        match self {
            Bid::Pass => false,
            Bid::OrderUp { alone } | Bid::Call { alone, .. } => alone,
        }
    }
}

impl Move {
    /// The stable wire id of this move (`moveId` in TS).
    pub fn id(self) -> LegalMoveId {
        match self {
            Move::Pass => "pass".to_owned(),
            Move::OrderUp { alone: false } => "orderUp".to_owned(),
            Move::OrderUp { alone: true } => "orderUp+alone".to_owned(),
            Move::Call { suit, alone: false } => format!("call:{}", suit.as_char()),
            Move::Call { suit, alone: true } => format!("call:{}+alone", suit.as_char()),
            Move::Cut { cut: true } => "cut:yes".to_owned(),
            Move::Cut { cut: false } => "cut:no".to_owned(),
            Move::Discard { card } => format!("discard:{card}"),
            Move::Play { card } => format!("play:{card}"),
        }
    }

    /// The bidding form of this move, when it belongs to the bidding vocabulary (V2).
    pub const fn as_bid(self) -> Option<Bid> {
        match self {
            Move::Pass => Some(Bid::Pass),
            Move::OrderUp { alone } => Some(Bid::OrderUp { alone }),
            Move::Call { suit, alone } => Some(Bid::Call { suit, alone }),
            Move::Cut { .. } | Move::Discard { .. } | Move::Play { .. } => None,
        }
    }

    /// Whether this move is `pass`, `orderUp` or `call` (`isBidMove` in TS).
    pub const fn is_bid(self) -> bool {
        self.as_bid().is_some()
    }

    /// The wire tag (`"pass"`, `"orderUp"`, `"call"`, `"cut"`, `"discard"`, `"play"`).
    pub const fn tag(self) -> &'static str {
        match self {
            Move::Pass => "pass",
            Move::OrderUp { .. } => "orderUp",
            Move::Call { .. } => "call",
            Move::Cut { .. } => "cut",
            Move::Discard { .. } => "discard",
            Move::Play { .. } => "play",
        }
    }
}

impl From<Bid> for Move {
    fn from(bid: Bid) -> Move {
        match bid {
            Bid::Pass => Move::Pass,
            Bid::OrderUp { alone } => Move::OrderUp { alone },
            Bid::Call { suit, alone } => Move::Call { suit, alone },
        }
    }
}

/// One offered move: its wire id, its structured form, and its human label.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct LegalMove {
    /// The wire id, e.g. `"call:S+alone"`.
    pub id: LegalMoveId,
    /// The structured move (JSON key `"move"`).
    #[serde(rename = "move")]
    pub mv: Move,
    /// UI/spoken label, e.g. `"I assist"`, `"Hearts, alone"`, `"Jack of diamonds"`.
    pub label: String,
}

/// An attributed move: what was played, by whom, and on whose behalf.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerAction {
    /// The acting seat.
    pub seat: Seat,
    /// The move's wire id.
    pub id: LegalMoveId,
    /// The structured move (JSON key `"move"`).
    #[serde(rename = "move")]
    pub mv: Move,
    /// How the move reached `apply`.
    pub kind: ActionKind,
    /// Present only when `kind == ai`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AiDecisionSource>,
    /// Present only when `kind == auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<AutoReason>,
}

/* -------------------------------------------------------------------------- */
/* Tricks, bidding records, dealing                                            */
/* -------------------------------------------------------------------------- */

/// One card laid by one seat.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Play {
    /// The seat that played.
    pub seat: Seat,
    /// The card laid.
    pub card: Card,
}

/// A trick, in progress or complete. `led_suit` is the **effective** suit of the first play.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trick {
    /// 0-based; a hand has exactly five tricks, indices `0..=4`.
    pub index: u8,
    /// `None` before any card is laid.
    pub led_suit: Option<Suit>,
    /// The plays so far, in order.
    pub plays: Vec<Play>,
    /// `None` until the trick resolves.
    pub winner_seat: Option<Seat>,
}

impl Trick {
    /// An empty trick at `index` (`emptyTrick` in TS).
    pub const fn empty(index: u8) -> Trick {
        Trick {
            index,
            led_suit: None,
            plays: Vec::new(),
            winner_seat: None,
        }
    }
}

/// One auction entry, kept for the talk log and the AI's bid-history memory.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct BidRecord {
    /// The bidding seat.
    pub seat: Seat,
    /// The bid (JSON key `"move"`).
    #[serde(rename = "move")]
    pub mv: Bid,
    /// The verbatim table line, e.g. `"I assist."`, `"Next."`, `"Turn it down."`.
    pub say: String,
}

/// One packet of the 3-2 / 2-3 alternation: `count` cards to `seat`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct DealPacket {
    /// The receiving seat.
    pub seat: Seat,
    /// How many cards (2 or 3).
    pub count: u8,
}

/// A deal packet on the choreography channel. `cards` is `None` (JSON `null`)
/// whenever the packet is not addressed to the receiving seat.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DealtPacket {
    /// The receiving seat.
    pub seat: Seat,
    /// How many cards (2 or 3).
    pub count: u8,
    /// The cards delivered, or `None` when redacted.
    pub cards: Option<Vec<Card>>,
}

/* -------------------------------------------------------------------------- */
/* Steps - the choreography channel                                            */
/* -------------------------------------------------------------------------- */

/// What just happened, so the client can animate and announce it. Decoration,
/// never truth. `DealerDiscarded` deliberately has no card field.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Step {
    /// `cut`: the cutter answered.
    Cut {
        /// The cutter.
        seat: Seat,
        /// `true` = bump.
        cut: bool,
    },
    /// `dealt`: the eight packets of the deal.
    Dealt {
        /// Packets in dealing order.
        packets: Vec<DealtPacket>,
    },
    /// `upCardTurned`: the up-card was turned face up.
    UpCardTurned {
        /// The up-card.
        card: Card,
    },
    /// `bid`: a seat bid.
    Bid {
        /// The bidder.
        seat: Seat,
        /// The bid's wire id.
        id: LegalMoveId,
        /// The verbatim table line.
        say: String,
    },
    /// `turnedDown`: four passes in round 1 buried the up-card.
    TurnedDown {
        /// The printed suit of the turned-down up-card.
        suit: Suit,
    },
    /// `trumpSet`: trump was named.
    TrumpSet {
        /// The trump suit.
        suit: Suit,
        /// The maker.
        maker_seat: Seat,
        /// The loner, if any.
        alone_seat: Option<Seat>,
        /// The loner's partner, if any.
        sitting_seat: Option<Seat>,
        /// `true` when trump came from an accepted up-card.
        via_order_up: bool,
    },
    /// `dealerDiscarded`: the dealer discarded (no card, by construction).
    DealerDiscarded {
        /// The dealer.
        seat: Seat,
    },
    /// `cardPlayed`: a card was laid.
    CardPlayed {
        /// The player.
        seat: Seat,
        /// The card.
        card: Card,
    },
    /// `trickWon`: a trick resolved.
    TrickWon {
        /// The trick index.
        index: u8,
        /// Who took it.
        winner_seat: Seat,
        /// The plays, in order.
        plays: Vec<Play>,
        /// Whether this trick makes the euchre certain.
        seals_euchre: bool,
    },
    /// `handScored`: the hand was scored.
    HandScored {
        /// The table row.
        result: HandResult,
        /// Post-clamp points by team.
        delta: [u8; 2],
        /// The new match score.
        score: [u8; 2],
        /// The maker.
        maker_seat: Option<Seat>,
        /// The loner, if any.
        alone_seat: Option<Seat>,
        /// Tricks by team.
        tricks_won: [u8; 2],
    },
    /// `throwIn`: four passes in round 2 with stick-the-dealer off.
    ThrowIn,
    /// `gameWon`: the match is over.
    GameWon {
        /// The winning team.
        team: Team,
        /// The final score.
        score: [u8; 2],
    },
}

impl Step {
    /// The step's `"t"` discriminant, e.g. `"upCardTurned"`.
    pub const fn tag(&self) -> &'static str {
        match self {
            Step::Cut { .. } => "cut",
            Step::Dealt { .. } => "dealt",
            Step::UpCardTurned { .. } => "upCardTurned",
            Step::Bid { .. } => "bid",
            Step::TurnedDown { .. } => "turnedDown",
            Step::TrumpSet { .. } => "trumpSet",
            Step::DealerDiscarded { .. } => "dealerDiscarded",
            Step::CardPlayed { .. } => "cardPlayed",
            Step::TrickWon { .. } => "trickWon",
            Step::HandScored { .. } => "handScored",
            Step::ThrowIn => "throwIn",
            Step::GameWon { .. } => "gameWon",
        }
    }
}

/* -------------------------------------------------------------------------- */
/* Configuration                                                               */
/* -------------------------------------------------------------------------- */

/// The deck variant. Only the 24-card deck exists.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum DeckVariant {
    /// `"24"`.
    #[default]
    #[serde(rename = "24")]
    TwentyFour,
}

/// The loner rule. Only the maker may go alone.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum LonerRule {
    /// `"makerOnly"`.
    #[default]
    #[serde(rename = "makerOnly")]
    MakerOnly,
}

/// The farmer's hand rule. It has no code path.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum FarmersHand {
    /// `"off"`.
    #[default]
    #[serde(rename = "off")]
    Off,
}

/// The rule variants the reducer branches on. Literal-typed TS fields are kept
/// so the JSON round-trips; only `game_to`, `stick_the_dealer`,
/// `require_natural_trump` and `misdeal_limit` change behaviour.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConfig {
    /// Always `"24"`.
    pub deck_variant: DeckVariant,
    /// Points to win. Default 10.
    pub game_to: u8,
    /// Default `true`: the dealer may not pass in round 2 once three have passed.
    pub stick_the_dealer: bool,
    /// A left bower alone does not qualify to name trump (TS optional field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_natural_trump: Option<bool>,
    /// Always `"makerOnly"`.
    pub loner: LonerRule,
    /// Always `false` in v1.
    pub defend_alone: bool,
    /// Always `false` in v1.
    pub super_euchre: bool,
    /// Always `"off"`.
    pub farmers_hand: FarmersHand,
    /// Always `false` in v1.
    pub railroading: bool,
    /// Always `false` in v1.
    pub dealer_must_hold_trump: bool,
    /// Cap on consecutive throw-ins tracked in `misdeal_streak`. Default 3.
    pub misdeal_limit: u8,
}

/// The v1 defaults (`DEFAULT_ENGINE_CONFIG` in TS).
pub const DEFAULT_ENGINE_CONFIG: EngineConfig = EngineConfig {
    deck_variant: DeckVariant::TwentyFour,
    game_to: 10,
    stick_the_dealer: true,
    require_natural_trump: Some(false),
    loner: LonerRule::MakerOnly,
    defend_alone: false,
    super_euchre: false,
    farmers_hand: FarmersHand::Off,
    railroading: false,
    dealer_must_hold_trump: false,
    misdeal_limit: 3,
};

impl Default for EngineConfig {
    fn default() -> Self {
        DEFAULT_ENGINE_CONFIG
    }
}

impl EngineConfig {
    /// Whether the natural-trump house rule is on (an absent flag means off).
    pub fn natural_trump_required(&self) -> bool {
        self.require_natural_trump.unwrap_or(false)
    }

    /// This config with every `Some` override applied (`{...cfg, ...overrides}`;
    /// unlike a JS spread, an absent override never overwrites).
    pub fn with_overrides(&self, o: &EngineConfigOverrides) -> EngineConfig {
        EngineConfig {
            deck_variant: o.deck_variant.unwrap_or(self.deck_variant),
            game_to: o.game_to.unwrap_or(self.game_to),
            stick_the_dealer: o.stick_the_dealer.unwrap_or(self.stick_the_dealer),
            require_natural_trump: o.require_natural_trump.or(self.require_natural_trump),
            loner: o.loner.unwrap_or(self.loner),
            defend_alone: o.defend_alone.unwrap_or(self.defend_alone),
            super_euchre: o.super_euchre.unwrap_or(self.super_euchre),
            farmers_hand: o.farmers_hand.unwrap_or(self.farmers_hand),
            railroading: o.railroading.unwrap_or(self.railroading),
            dealer_must_hold_trump: o
                .dealer_must_hold_trump
                .unwrap_or(self.dealer_must_hold_trump),
            misdeal_limit: o.misdeal_limit.unwrap_or(self.misdeal_limit),
        }
    }
}

/// `Partial<EngineConfig>`: rule overrides on top of [`DEFAULT_ENGINE_CONFIG`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConfigOverrides {
    /// Override for `deckVariant`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deck_variant: Option<DeckVariant>,
    /// Override for `gameTo`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_to: Option<u8>,
    /// Override for `stickTheDealer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stick_the_dealer: Option<bool>,
    /// Override for `requireNaturalTrump`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_natural_trump: Option<bool>,
    /// Override for `loner`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loner: Option<LonerRule>,
    /// Override for `defendAlone`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defend_alone: Option<bool>,
    /// Override for `superEuchre`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub super_euchre: Option<bool>,
    /// Override for `farmersHand`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub farmers_hand: Option<FarmersHand>,
    /// Override for `railroading`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub railroading: Option<bool>,
    /// Override for `dealerMustHoldTrump`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dealer_must_hold_trump: Option<bool>,
    /// Override for `misdealLimit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub misdeal_limit: Option<u8>,
}

/* -------------------------------------------------------------------------- */
/* State                                                                       */
/* -------------------------------------------------------------------------- */

/// All four hands, indexed by seat. Serialized as the TS
/// `Record<Seat, CardId[]>` object `{"0":[..],"1":[..],"2":[..],"3":[..]}`.
pub type Hands = [Vec<Card>; 4];

/// Serde for [`Hands`] as a JSON object keyed `"0".."3"` (use with `#[serde(with)]`).
pub mod hands_serde {
    use super::*;

    const KEYS: [&str; 4] = ["0", "1", "2", "3"];

    /// Serializes the four hands as `{"0":[..],"1":[..],"2":[..],"3":[..]}`.
    pub fn serialize<S: Serializer>(hands: &Hands, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(4))?;
        for (key, cards) in KEYS.iter().zip(hands.iter()) {
            map.serialize_entry(key, cards)?;
        }
        map.end()
    }

    /// Deserializes the `"0".."3"` object; every key is required exactly once.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Hands, D::Error> {
        struct HandsVisitor;

        impl<'de> Visitor<'de> for HandsVisitor {
            type Value = Hands;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object with keys \"0\", \"1\", \"2\" and \"3\"")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Hands, A::Error> {
                let mut out: [Option<Vec<Card>>; 4] = [None, None, None, None];
                while let Some(key) = map.next_key::<String>()? {
                    let i = KEYS
                        .iter()
                        .position(|k| *k == key)
                        .ok_or_else(|| de::Error::unknown_field(&key, &KEYS))?;
                    if out[i].is_some() {
                        return Err(de::Error::duplicate_field(KEYS[i]));
                    }
                    out[i] = Some(map.next_value()?);
                }
                let [a, b, c, d] = out;
                Ok([
                    a.ok_or_else(|| de::Error::missing_field("0"))?,
                    b.ok_or_else(|| de::Error::missing_field("1"))?,
                    c.ok_or_else(|| de::Error::missing_field("2"))?,
                    d.ok_or_else(|| de::Error::missing_field("3"))?,
                ])
            }
        }

        deserializer.deserialize_map(HandsVisitor)
    }
}

/// Everything that belongs to the hand in progress. Reset wholesale by `start_hand`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandState {
    /// 0-based hand counter within the match.
    pub hand_no: u32,
    /// The current phase.
    pub phase: GamePhase,
    /// The dealer.
    pub dealer_seat: Seat,
    /// `None` in every engine-only phase.
    pub turn_seat: Option<Seat>,
    /// The persisted 24-card permutation (post-cut after a bump).
    pub deck_order: Vec<Card>,
    /// `None` until the cutter answers; `true` = bump.
    pub cut: Option<bool>,
    /// All four hands (server-only; foreign hands never reach a view).
    #[serde(with = "hands_serde")]
    pub hands: Hands,
    /// `deck_order[20..24]`; `kitty[0]` is the up-card.
    pub kitty: [Card; 4],
    /// `kitty[0]` once dealt; never nulled (V14).
    pub up_card: Option<Card>,
    /// `true` after the round-1 four-pass.
    pub up_card_turned_down: bool,
    /// Printed suit of the turned-down up-card.
    pub turned_down_suit: Option<Suit>,
    /// Trump, once named.
    pub trump: Option<Suit>,
    /// The maker, once trump is named.
    pub maker_seat: Option<Seat>,
    /// The loner; non-null implies `== maker_seat`.
    pub alone_seat: Option<Seat>,
    /// The loner's partner.
    pub sitting_seat: Option<Seat>,
    /// The dealer's face-down discard; never revealed.
    pub dealer_discard: Option<Card>,
    /// Passes within the current round; `<= 4`.
    pub passes: u8,
    /// The auction so far; `<= 8` entries.
    pub bids: Vec<BidRecord>,
    /// The current (or just-completed) trick.
    pub trick: Trick,
    /// Completed tricks, oldest first; `<= 5`.
    pub trick_log: Vec<Trick>,
    /// Tricks by team.
    pub tricks_won: [u8; 2],
    /// Set at `hand_score`.
    pub result: Option<HandResult>,
    /// Post-clamp points awarded by this hand.
    pub delta: Option<[u8; 2]>,
}

/// An early AI decision parked behind the persisted pacing floor (actor-owned).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDecision {
    /// The AI seat.
    pub seat: Seat,
    /// The turn the decision was requested for.
    pub turn_id: String,
    /// Epoch ms when the decision was requested.
    pub requested_at: u64,
    /// Epoch ms before which the decision may not be revealed.
    pub reveal_at: u64,
    /// The decided move id, once known.
    pub decision: Option<LegalMoveId>,
    /// Where the decision came from, once known.
    pub source: Option<AiDecisionSource>,
    /// A parked safety move is not an answer. Older tables omit this flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered: Option<bool>,
}

/// The `schema` field of [`GameState`]: always the JSON number `1`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct SchemaV1;

impl TryFrom<u8> for SchemaV1 {
    type Error = OutOfRange;

    fn try_from(n: u8) -> Result<Self, Self::Error> {
        if n == 1 {
            Ok(SchemaV1)
        } else {
            Err(OutOfRange {
                type_name: "schema",
                value: n,
            })
        }
    }
}

impl From<SchemaV1> for u8 {
    fn from(_: SchemaV1) -> u8 {
        1
    }
}

/// The whole authoritative match state. **Server-only:** it carries the seed,
/// every hand and the `internal_token`, so it must never be sent to a client.
/// It is serializable only for actor persistence; clients get [`PublicGameView`]
/// via `project`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameState {
    /// Always `1`.
    pub schema: SchemaV1,
    /// Monotonic version; actor-owned (the reducer never reads or writes it).
    pub v: u64,
    /// The match id.
    pub game_id: String,
    /// The actor-to-actor authorization secret.
    pub internal_token: String,
    /// The match seed; the RNG is rebuilt from it.
    pub seed: String,
    /// The rule variants.
    pub cfg: EngineConfig,
    /// The dealer of hand 0.
    pub first_dealer: Seat,
    /// Match status.
    pub status: GameStatus,
    /// Match score by team.
    pub score: [u8; 2],
    /// 0-based; increments on every scored hand (except the last) and every throw-in.
    pub hand_no: u32,
    /// Consecutive throw-ins, capped at `cfg.misdeal_limit`.
    pub misdeal_streak: u8,
    /// Set exactly when a team reaches `cfg.game_to`.
    pub winner_team: Option<Team>,
    /// The hand in progress.
    pub hand: HandState,
    /// Server turn nonce, regenerated on every transition.
    pub turn_id: String,
    /// Actor-owned turn deadline (epoch ms).
    pub turn_deadline_at: Option<u64>,
    /// Actor-owned schedule id.
    pub timer_id: Option<String>,
    /// Actor-owned schedule id.
    pub watchdog_id: Option<String>,
    /// Actor-owned schedule id.
    pub reveal_id: Option<String>,
    /// Actor-owned parked AI decision.
    pub pending: Option<PendingDecision>,
    /// Actor-owned ring buffer for double-click dedupe (cap 32).
    pub recent_move_ids: Vec<String>,
    /// Actor-owned idempotency counter.
    pub applied_seq: u64,
}

/* -------------------------------------------------------------------------- */
/* Projection                                                                  */
/* -------------------------------------------------------------------------- */

string_enum! {
    /// The deciding phase a [`TableObservation`] was recorded in.
    ObservationKind {
        /// `"cut"`.
        Cut => "cut",
        /// `"bid1"`.
        Bid1 => "bid1",
        /// `"bid2"`.
        Bid2 => "bid2",
        /// `"discard"`.
        Discard => "discard",
        /// `"play"`.
        Play => "play",
    }
}

string_enum! {
    /// How trustworthy a [`TableObservation`]'s timing is.
    ObservationTiming {
        /// `"human-observed"`.
        HumanObserved => "human-observed",
        /// `"computer-paced"`.
        ComputerPaced => "computer-paced",
        /// `"unreliable"`.
        Unreliable => "unreliable",
    }
}

/// Public action timing: human/network delays, never hidden AI reasoning.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableObservation {
    /// The hand it was observed in.
    pub hand_no: u32,
    /// The acting seat.
    pub seat: Seat,
    /// The deciding phase.
    pub kind: ObservationKind,
    /// Public description of the action.
    pub action: String,
    /// Rounded elapsed time, or `None` when unreliable or computer-paced.
    pub elapsed_ms: Option<u64>,
    /// Timing provenance.
    pub timing: ObservationTiming,
}

/// Public persona metadata for one AI seat.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TablePlayer {
    /// The seat.
    pub seat: Seat,
    /// Display name.
    pub name: String,
    /// One-line description.
    pub blurb: String,
    /// Play-style label.
    pub style: String,
}

/// The public rule flags the actor attaches to a view.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRules {
    /// `cfg.stickTheDealer`.
    pub stick_the_dealer: bool,
    /// `cfg.requireNaturalTrump`.
    pub require_natural_trump: bool,
}

/// The client view built by `project(state, seat)`, plus optional actor metadata.
///
/// Redaction is structural: there is no field capable of holding a foreign hand,
/// a buried kitty card or the dealer's discard (V12). Never widen it.
///
/// Field order is the TS wire key order, with the actor metadata after
/// `winner_team`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicGameView {
    /// The state version this view was cut from.
    pub v: u64,
    /// The seat this view was projected for.
    pub you: Seat,
    /// Match status.
    pub status: GameStatus,
    /// Current phase.
    pub phase: GamePhase,
    /// The **hand's** hand number.
    pub hand_no: u32,
    /// The dealer.
    pub dealer_seat: Seat,
    /// The seat to act, if any.
    pub turn_seat: Option<Seat>,
    /// Echoed back on `submitMove`.
    pub turn_id: String,
    /// Match score by team.
    pub score: [u8; 2],
    /// Trump, once named.
    pub trump: Option<Suit>,
    /// The maker.
    pub maker_seat: Option<Seat>,
    /// The loner.
    pub alone_seat: Option<Seat>,
    /// The loner's partner.
    pub sitting_seat: Option<Seat>,
    /// The up-card; never nulled once dealt (V14).
    pub up_card: Option<Card>,
    /// Distinguishes buried from picked up.
    pub up_card_turned_down: bool,
    /// The suit that may not be named in round 2.
    pub turned_down_suit: Option<Suit>,
    /// Your own cards, and only ever your own. Empty for the sitting seat.
    pub hand: Vec<Card>,
    /// Card counts by seat; `0` for the sitting seat.
    pub hand_counts: [u8; 4],
    /// `3 + (up_card_turned_down || dealer_discard.is_some() ? 1 : 0)`.
    pub kitty_count: u8,
    /// The current trick.
    pub trick: Trick,
    /// Completed tricks, oldest first.
    pub trick_log: Vec<Trick>,
    /// Tricks by team this hand.
    pub tricks_won: [u8; 2],
    /// Passes within the current round.
    pub passes: u8,
    /// The auction so far.
    pub bids: Vec<BidRecord>,
    /// Empty unless `turn_seat == you`; never empty when it is (V19).
    pub legal: Vec<LegalMove>,
    /// Set at `hand_score`.
    pub result: Option<HandResult>,
    /// Post-clamp points of this hand.
    pub delta: Option<[u8; 2]>,
    /// Set when a team reaches `cfg.game_to`.
    pub winner_team: Option<Team>,
    // The actor metadata comes last: the TS table builds its view as
    // `{ ...project(game, seat), players, observations, ... }`.
    /// Actor-authored persona metadata (absent from the pure projection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub players: Option<Vec<TablePlayer>>,
    /// Actor-authored public timing log (absent from the pure projection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observations: Option<Vec<TableObservation>>,
    /// Actor-owned accessibility preference (absent from the pure projection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_tricks: Option<bool>,
    /// Public rule flags (absent from the pure projection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<TableRules>,
    /// True only after the server-held read pause (absent from the pure projection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awaiting_trick_review: Option<bool>,
}
