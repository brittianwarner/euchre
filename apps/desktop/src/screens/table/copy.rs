//! Every sentence the table shows, as pure functions of the view (06 §5, §9, §12). Nothing here
//! draws; the strings are verbatim from the spec, typographic apostrophes and all, so the unit
//! tests below pin them.

use euchre_engine::{WhyIllegalParams, card_name, suit_name, why_illegal};
use euchre_protocol::TableStore;
use euchre_protocol::{
    Bid, Card, GamePhase, GameStatus, HandResult, LegalMove, Move, PublicGameView, Seat, Step,
    Suit, Team,
};

/// Hand heading while a move is in flight.
pub const H_SENDING: &str = "Sending your move…";
/// Hand heading once the match is over.
pub const H_OVER: &str = "A good game, well played.";
/// Hand heading while a finished trick waits for Continue.
pub const H_REVIEW: &str = "Review the trick.";
/// Hand heading on the player's turn.
pub const H_TURN: &str = "Your turn.";
/// The hand heading while the table is paused (its connection ended).
pub const H_PAUSED: &str = "Paused.";
/// The hand area's instruction while it is paused.
pub const PAUSED_LINE: &str = "Play picks up where it left off.";
/// Hand heading otherwise.
pub const H_IDLE: &str = "Make yourself at home.";
/// The hand area's status while the deal is on its way (06 §14.1's dealing line).
pub const DEALING: &str = "Dealing your cards…";
/// A seat's detail while the deal is on its way: its count would run ahead of the backs
/// still flying to it.
pub const SEAT_DEALING: &str = "Dealing…";
/// The end-of-match action.
pub const PLAY_AGAIN: &str = "Play again →";
/// The instruction while a move is in flight.
pub const WAITING_FOR_CONFIRM: &str = "Waiting for the table to confirm your move…";
/// The Continue button (06 §5.4.5).
pub const CONTINUE: &str = "Continue →";
/// The Continue button while the trick gathers.
pub const COLLECTING: &str = "Collecting… →";
/// The Go alone checkbox.
pub const GO_ALONE: &str = "Go alone";
/// Banner after a lone march (06 §9).
pub const BANNER_LONE_MARCH: &str = "All five alone! Four points.";
/// Banner when your team wins the match.
pub const BANNER_YOU_WIN: &str = "Your team wins!";
/// Banner when the other team wins.
pub const BANNER_THEY_WIN: &str = "A good game. Well played.";
/// The `First to 10` caption.
pub const FIRST_TO: &str = "First to 10";
/// The rack's legal-card hint (06 §5.6).
pub const RACK_LEGAL_HINT: &str = "Press to select, then use Play. Or double-click to play.";

/// Seat position relative to the viewer: 0 you, 1 left, 2 partner, 3 right.
pub fn rel(view: &PublicGameView, seat: Seat) -> usize {
    (seat.index() + 4 - view.you.index()) % 4
}

/// The deal's packets in dealing order, from the dealer's left: 3-2-3-2, then 2-3-2-3.
const DEAL_PACKETS: [usize; 8] = [3, 2, 3, 2, 2, 3, 2, 3];

/// Where `seat`'s `k`th card (0-based, 0..5) falls in the whole deal (09 §4.1): its global
/// index 0..20 for `motion::deal_delay`, dealing in packets from `dealer`'s left.
pub fn deal_index(dealer: Seat, seat: Seat, k: usize) -> usize {
    let pos = (seat.index() + 3 - dealer.index()) % 4;
    let first = DEAL_PACKETS[pos];
    let packet = if k < first { pos } else { 4 + pos };
    let within = if k < first { k } else { k - first };
    let before: usize = DEAL_PACKETS[..packet].iter().sum();
    (before + within).min(19)
}

/// The seat at relative position `rel_pos`.
pub fn seat_at(view: &PublicGameView, rel_pos: usize) -> Seat {
    Seat::ALL[(view.you.index() + rel_pos) % 4]
}

/// `seatName` (06 §5.4.2, D16).
pub fn seat_name(view: &PublicGameView, seat: Seat) -> &'static str {
    ["You", "Left opponent", "Your partner", "Right opponent"][rel(view, seat)]
}

/// The short role word in a seat's detail line.
pub fn role_word(view: &PublicGameView, seat: Seat) -> &'static str {
    ["You", "Left", "Partner", "Right"][rel(view, seat)]
}

/// `playerName`: You, the persona's name, or the seat name.
pub fn player_name(view: &PublicGameView, seat: Seat) -> String {
    if seat == view.you {
        return "You".to_owned();
    }
    view.players
        .as_ref()
        .and_then(|ps| ps.iter().find(|p| p.seat == seat))
        .map_or_else(|| seat_name(view, seat).to_owned(), |p| p.name.clone())
}

/// The persona blurb behind a name (its tooltip), if the table sent one.
pub fn player_blurb(view: &PublicGameView, seat: Seat) -> Option<String> {
    view.players
        .as_ref()
        .and_then(|ps| ps.iter().find(|p| p.seat == seat))
        .map(|p| p.blurb.clone())
        .filter(|b| !b.is_empty())
}

/// `are` for You, `is` for everyone else.
fn is_are(view: &PublicGameView, seat: Seat) -> &'static str {
    if seat == view.you { "are" } else { "is" }
}

/// The plain verb for You, the `-s` verb for everyone else.
fn verb(
    view: &PublicGameView,
    seat: Seat,
    plain: &'static str,
    third: &'static str,
) -> &'static str {
    if seat == view.you { plain } else { third }
}

/// The suit glyph.
pub fn glyph(suit: Suit) -> &'static str {
    match suit {
        Suit::Hearts => "♥",
        Suit::Diamonds => "♦",
        Suit::Spades => "♠",
        Suit::Clubs => "♣",
    }
}

/// The bidding phases, while the felt shows the bid area instead of the trick (06 §5.4.3).
pub fn is_bidding(phase: GamePhase) -> bool {
    matches!(
        phase,
        GamePhase::Lobby
            | GamePhase::Cutting
            | GamePhase::Deal
            | GamePhase::BidRound1
            | GamePhase::BidRound2
            | GamePhase::DealerDiscard
    )
}

/// Phases in which the bid panel can show (06 §5.8).
pub fn is_bid_panel_phase(phase: GamePhase) -> bool {
    matches!(
        phase,
        GamePhase::Cutting | GamePhase::BidRound1 | GamePhase::BidRound2
    )
}

/// Whether completed tricks wait for Continue.
pub fn review_mode(view: &PublicGameView) -> bool {
    view.review_tricks == Some(true)
}

/// `{Rank of suit}` played by `seat` in the trick, as the card's accessible name.
pub fn played_label(view: &PublicGameView, seat: Seat, card: Card) -> String {
    let won = view.trick.winner_seat == Some(seat);
    format!(
        "{} played {}{}",
        seat_name(view, seat),
        card_name(card),
        if won { " — winning card" } else { "" }
    )
}

/// `{n} points, counted with a six and a four` (06 §5.3).
pub fn score_cards_label(score: u8) -> String {
    format!("{score} points, counted with a six and a four")
}

/// The status row's trump line: `{glyph} {Suit} are trump` or `Choosing trump`, as
/// (glyph, words).
pub fn trump_line(view: &PublicGameView) -> (Option<&'static str>, String) {
    trump_words(view.trump)
}

/// The trump line for `trump`: its glyph and `{Suit} are trump`, or `Choosing trump`.
pub fn trump_words(trump: Option<Suit>) -> (Option<&'static str>, String) {
    match trump {
        Some(s) => (Some(glyph(s)), format!("{} are trump", suit_name(s))),
        None => (None, "Choosing trump".to_owned()),
    }
}

/// `★ {seat} alone`.
pub fn alone_line(view: &PublicGameView) -> Option<String> {
    view.alone_seat
        .map(|s| format!("★ {} alone", seat_name(view, s)))
}

/// The seat that leads this trick: its first play, or the turn seat at the start of play.
pub fn lead_seat(view: &PublicGameView) -> Option<Seat> {
    view.trick.plays.first().map(|p| p.seat).or_else(|| {
        (view.phase == GamePhase::TrickPlay)
            .then_some(view.turn_seat)
            .flatten()
    })
}

/// Whether `seat`'s latest bid in the current round was a pass.
pub fn latest_bid_passed(view: &PublicGameView, seat: Seat) -> bool {
    let round: &[euchre_protocol::BidRecord] = match view.phase {
        GamePhase::BidRound1 => &view.bids,
        GamePhase::BidRound2 => view.bids.get(4..).unwrap_or(&[]),
        _ => return false,
    };
    round
        .iter()
        .rev()
        .find(|b| b.seat == seat)
        .is_some_and(|b| matches!(b.mv, Bid::Pass))
}

/// The seat detail line (06 §5.4.2) after its `{Role} · ` prefix.
pub fn seat_detail(view: &PublicGameView, seat: Seat, thinking: bool) -> String {
    let turn = view.turn_seat == Some(seat);
    if view.sitting_seat == Some(seat) {
        return "Sitting out · partner alone".to_owned();
    }
    if turn && seat == view.you {
        return "Your turn".to_owned();
    }
    if (turn || thinking) && seat != view.you {
        return "Thinking…".to_owned();
    }
    if is_bid_panel_phase(view.phase) && latest_bid_passed(view, seat) {
        return "Passed".to_owned();
    }
    seat_count(view, seat)
}

/// A seat's card count ("5 cards", "1 card", 06 §5.4.2).
pub fn seat_count(view: &PublicGameView, seat: Seat) -> String {
    match view.hand_counts[seat.index()] {
        1 => "1 card".to_owned(),
        n => format!("{n} cards"),
    }
}

/// `turnInstruction(view)` (06 §12), first match wins.
pub fn turn_instruction(view: &PublicGameView) -> String {
    match view.status {
        GameStatus::Abandoned => {
            return "This game has ended. Start a new game whenever you’re ready.".to_owned();
        }
        GameStatus::Complete => {
            return if you_won(view) {
                "Your team wins the match!".to_owned()
            } else {
                "The other team wins the match.".to_owned()
            };
        }
        GameStatus::Active => {}
    }
    if view.phase == GamePhase::HandScore {
        return "Hand complete. Adding the points…".to_owned();
    }
    if view.awaiting_trick_review == Some(true) {
        return if view.trick.index == 4 {
            "Press Continue to finish this hand.".to_owned()
        } else if view.trick.winner_seat == Some(view.you) {
            "You lead next. Press Continue first.".to_owned()
        } else {
            "Press Continue when you’re ready for the next trick.".to_owned()
        };
    }
    if view.phase == GamePhase::TrickResolve {
        return match view.trick.winner_seat {
            None => "Finishing this trick…".to_owned(),
            Some(w) => format!(
                "{} {} the trick.",
                seat_name(view, w),
                verb(view, w, "win", "wins")
            ),
        };
    }
    if view.sitting_seat == Some(view.you) {
        return "Your partner is going alone. Enjoy the hand.".to_owned();
    }
    match view.turn_seat {
        None => return "Getting the table ready…".to_owned(),
        Some(seat) if seat != view.you => {
            let doing = if matches!(view.phase, GamePhase::BidRound1 | GamePhase::BidRound2) {
                "deciding on trump"
            } else {
                "taking a turn"
            };
            return format!("{} is {doing}…", seat_name(view, seat));
        }
        Some(_) => {}
    }
    match view.phase {
        GamePhase::DealerDiscard => "You’re the dealer. Select one card to discard.".to_owned(),
        GamePhase::Cutting => "Your choice: cut the deck, or let the dealer deal.".to_owned(),
        GamePhase::BidRound1 => match view.up_card {
            Some(up) => format!(
                "Make {} trump, or pass.",
                suit_name(up.printed_suit()).to_lowercase()
            ),
            None => "Choose a trump suit, or pass.".to_owned(),
        },
        GamePhase::BidRound2 => {
            if view.legal.iter().any(|m| m.id == "pass") {
                "Choose a trump suit, or pass.".to_owned()
            } else {
                "Everyone passed. Stick the dealer is on: choose trump.".to_owned()
            }
        }
        _ => match view.trick.led_suit {
            Some(s) => format!("{} were led. Follow suit if you can.", suit_name(s)),
            None => "You lead. Select any card, then press Play.".to_owned(),
        },
    }
}

/// Whether the viewer's team won the match.
pub fn you_won(view: &PublicGameView) -> bool {
    view.winner_team.map(Team::index) == Some(view.you.index() % 2)
}

/// The hand heading (06 §5.5), first match wins.
pub fn hand_heading(view: &PublicGameView, submitting: bool) -> &'static str {
    if submitting {
        H_SENDING
    } else if view.status != GameStatus::Active {
        H_OVER
    } else if view.phase == GamePhase::TrickResolve && review_mode(view) {
        H_REVIEW
    } else if view.turn_seat == Some(view.you) && view.phase != GamePhase::TrickResolve {
        H_TURN
    } else {
        H_IDLE
    }
}

/// The legal `play:` or `discard:` move for `card`, if any.
pub fn card_move(view: &PublicGameView, card: Card) -> Option<&LegalMove> {
    view.legal.iter().find(|m| match m.mv {
        Move::Play { card: c } | Move::Discard { card: c } => c == card,
        _ => false,
    })
}

/// Whether the view offers any card move.
pub fn has_card_moves(view: &PublicGameView) -> bool {
    view.legal
        .iter()
        .any(|m| matches!(m.mv, Move::Play { .. } | Move::Discard { .. }))
}

fn why(view: &PublicGameView, card: Card) -> Option<String> {
    why_illegal(WhyIllegalParams {
        hand: &view.hand,
        card,
        led_suit: view.trick.led_suit,
        trump: view.trump,
    })
}

/// `illegalReason(view, card)` (06 §5.7): the message a tap on an unavailable card shows.
pub fn illegal_reason(view: &PublicGameView, card: Card) -> String {
    if view.status != GameStatus::Active {
        return "This game is finished.".to_owned();
    }
    if view.phase == GamePhase::TrickResolve && review_mode(view) {
        return "Press Continue to finish reviewing this trick before playing.".to_owned();
    }
    if view.turn_seat != Some(view.you) {
        return "It's not your turn yet.".to_owned();
    }
    if view.phase == GamePhase::DealerDiscard {
        return "It's not your turn to discard.".to_owned();
    }
    if view.phase != GamePhase::TrickPlay {
        return "Finish bidding before you play a card.".to_owned();
    }
    why(view, card).unwrap_or_else(|| "That card can't be played right now.".to_owned())
}

/// `unavailableReason(card)` (06 §5.6): the accessible reason on an unavailable card.
pub fn unavailable_reason(view: &PublicGameView, card: Card, disabled: bool) -> String {
    if disabled {
        return "Please wait until the table is ready.".to_owned();
    }
    if view.status != GameStatus::Active {
        return "This match is finished.".to_owned();
    }
    if view.phase == GamePhase::TrickResolve && review_mode(view) {
        return "Press Continue to finish reviewing this trick.".to_owned();
    }
    if view.turn_seat != Some(view.you) {
        return "Wait for your turn.".to_owned();
    }
    if !matches!(view.phase, GamePhase::TrickPlay | GamePhase::DealerDiscard) {
        return "Finish bidding before selecting a card.".to_owned();
    }
    why(view, card).unwrap_or_else(|| "This card is not available right now.".to_owned())
}

/// A rack card's accessible name.
pub fn rack_label(view: &PublicGameView, card: Card, legal: bool, disabled: bool) -> String {
    format!(
        "Select {}. {}",
        card_name(card),
        if legal {
            RACK_LEGAL_HINT.to_owned()
        } else {
            unavailable_reason(view, card, disabled)
        }
    )
}

/// The confirm button: `{Playing… | Discard | Play} {card | selected card} →` (06 §5.5).
pub fn play_button_label(
    view: &PublicGameView,
    selected: Option<Card>,
    submitting: bool,
) -> String {
    let verb = if submitting {
        "Playing…"
    } else if view.phase == GamePhase::DealerDiscard {
        "Discard"
    } else {
        "Play"
    };
    let what = selected.map_or_else(|| "selected card".to_owned(), card_name);
    format!("{verb} {what} →")
}

/// A bid button's words (06 §5.8).
pub fn bid_label(view: &PublicGameView, m: &LegalMove) -> String {
    match m.mv {
        Move::Pass => "Pass".to_owned(),
        Move::OrderUp { .. } => match view.up_card {
            Some(up) => format!("Make {} trump", suit_name(up.printed_suit()).to_lowercase()),
            None => "Make it trump".to_owned(),
        },
        Move::Call { suit, .. } => suit_name(suit).to_owned(),
        Move::Cut { cut: true } => "Cut the deck".to_owned(),
        Move::Cut { cut: false } => "Deal the cards".to_owned(),
        Move::Play { .. } | Move::Discard { .. } => m.label.clone(),
    }
}

/// Whether a bid button takes the secondary style (`pass`, `cut:no`).
pub fn bid_is_secondary(m: &LegalMove) -> bool {
    matches!(m.mv, Move::Pass | Move::Cut { cut: false })
}

/// The bid area's copy (06 §5.4.4): eyebrow, heading and line.
pub struct BidAreaCopy {
    /// The eyebrow.
    pub eyebrow: &'static str,
    /// The heading.
    pub heading: String,
    /// The paragraph.
    pub line: String,
}

/// Whether the up card shows face up (06 §5.4.4 case A).
pub fn up_card_showing(view: &PublicGameView) -> Option<Card> {
    view.up_card
        .filter(|_| !view.up_card_turned_down && view.phase != GamePhase::DealerDiscard)
}

/// The bid area's copy for the view.
pub fn bid_area_copy(view: &PublicGameView) -> BidAreaCopy {
    let dealer = seat_name(view, view.dealer_seat);
    if let Some(up) = up_card_showing(view) {
        return BidAreaCopy {
            eyebrow: "The up card",
            heading: card_name(up),
            line: format!(
                "{dealer} {} it up if its suit becomes trump.",
                verb(view, view.dealer_seat, "pick", "picks")
            ),
        };
    }
    let up = view
        .up_card
        .map_or_else(|| "the up card".to_owned(), card_name);
    let heading = match view.phase {
        GamePhase::BidRound2 => format!("{dealer} turned down {up}."),
        GamePhase::DealerDiscard => format!("{dealer} picked up {up}."),
        _ => "A fresh hand awaits.".to_owned(),
    };
    let line = match (view.phase, view.turned_down_suit) {
        (GamePhase::BidRound2, Some(s)) => {
            format!("{} were turned down. Choose another suit.", suit_name(s))
        }
        _ => "You and your partner play together. First team to 10 wins.".to_owned(),
    };
    BidAreaCopy {
        eyebrow: "A seat at the table",
        heading,
        line,
    }
}

/// The trick result line (06 §5.4.5).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResultLine {
    /// `{who} {win|wins} the trick.` and what happens next.
    Won {
        /// The strong part.
        strong: String,
        /// The next-lead part.
        next: String,
    },
    /// A banter line: the speaker and the quote.
    Talk {
        /// The speaker's display name.
        speaker: String,
        /// The quote, in curly quotes.
        quote: String,
    },
    /// Any other line.
    Plain(String),
}

/// The result line for the view, with the visible banter (`talk`) if any.
pub fn trick_result(view: &PublicGameView, talk: Option<(Seat, &str)>) -> ResultLine {
    if let Some(winner) = view.trick.winner_seat {
        let who = seat_name(view, winner);
        let next = if view.trick.index >= 4 {
            "That’s the last trick of this hand.".to_owned()
        } else if review_mode(view) {
            "Press Continue before the next lead.".to_owned()
        } else {
            format!("{who} {} next.", verb(view, winner, "lead", "leads"))
        };
        return ResultLine::Won {
            strong: format!("{who} {} the trick.", verb(view, winner, "win", "wins")),
            next,
        };
    }
    if let Some((seat, text)) = talk {
        return ResultLine::Talk {
            speaker: player_name(view, seat),
            quote: format!("“{text}”"),
        };
    }
    if let Some(alone) = view.alone_seat {
        return ResultLine::Plain(format!(
            "{} {} going alone.",
            seat_name(view, alone),
            is_are(view, alone)
        ));
    }
    if is_bidding(view.phase) {
        ResultLine::Plain("Take your time. There’s no rush.".to_owned())
    } else {
        ResultLine::Plain(format!(
            "Trick {} of 5 · The winner leads the next trick.",
            view.trick.index + 1
        ))
    }
}

/// `seat`'s banter line while it shows (06 §8: banter only, this hand only, 6.5 s), for the
/// bubble under the seat.
pub fn seat_banter(store: &TableStore, seat: Seat) -> Option<&str> {
    store
        .visible_table_talk()
        .filter(|t| t.seat == seat)
        .map(|t| t.text.as_str())
}

/// The hand-result card's copy (09 §4.8): the result word and who scored, in the announcer's
/// words (06 §10), the points, and the makers' tricks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandResultCopy {
    /// `Point.`, `March!`, `Euchre!` and so on.
    pub title: &'static str,
    /// `You score {n}.`, `They score {n}.` or `Nobody scores.`
    pub line: String,
    /// The points the scoring side took (0 when nobody scores).
    pub points: u8,
    /// The makers' tricks of the five, when someone called trump.
    pub maker_tricks: Option<u8>,
}

/// The card's copy while the hand scores (`hand_score` with a result), else `None`.
pub fn hand_result(view: &PublicGameView) -> Option<HandResultCopy> {
    let result = view.result.filter(|_| view.phase == GamePhase::HandScore)?;
    let mine = view.you.index() % 2;
    let delta = view.delta.unwrap_or([0, 0]);
    let (line, points) = if delta[mine] > 0 {
        (format!("You score {}.", delta[mine]), delta[mine])
    } else if delta[1 - mine] > 0 {
        (format!("They score {}.", delta[1 - mine]), delta[1 - mine])
    } else {
        ("Nobody scores.".to_owned(), 0)
    };
    Some(HandResultCopy {
        title: super::announcer::result_label(result),
        line,
        points,
        maker_tricks: view.maker_seat.map(|s| view.tricks_won[s.index() % 2]),
    })
}

/// An outcome banner (06 §9): its text and whether confetti falls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerCopy {
    /// The line after the star.
    pub text: String,
    /// Confetti with it.
    pub confetti: bool,
}

/// The banner a step raises, if any.
pub fn banner_for(view: &PublicGameView, step: &Step) -> Option<BannerCopy> {
    match step {
        Step::TrumpSet {
            alone_seat: Some(alone),
            ..
        } => Some(BannerCopy {
            text: format!(
                "{} {} going alone!",
                seat_name(view, *alone),
                is_are(view, *alone)
            ),
            confetti: false,
        }),
        Step::HandScored {
            result: HandResult::LoneMarch,
            ..
        } => Some(BannerCopy {
            text: BANNER_LONE_MARCH.to_owned(),
            confetti: true,
        }),
        Step::GameWon { team, .. } => {
            let ours = team.index() == view.you.index() % 2;
            Some(BannerCopy {
                text: if ours {
                    BANNER_YOU_WIN
                } else {
                    BANNER_THEY_WIN
                }
                .to_owned(),
                confetti: ours,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::TableConfig;
    use crate::net::sim::fixture_view;

    fn view() -> PublicGameView {
        fixture_view("copy-tests", TableConfig::default()).expect("fixture")
    }

    #[test]
    fn deal_index_follows_the_packets_from_the_dealers_left() {
        let [a, b, c, d] = Seat::ALL;
        // Dealer a: b takes 0..3, c 3..5, d 5..8, a 8..10, then b 10..12 and so on.
        assert_eq!(
            (0..5).map(|k| deal_index(a, b, k)).collect::<Vec<_>>(),
            [0, 1, 2, 10, 11]
        );
        assert_eq!(
            (0..5).map(|k| deal_index(a, c, k)).collect::<Vec<_>>(),
            [3, 4, 12, 13, 14]
        );
        assert_eq!(
            (0..5).map(|k| deal_index(a, d, k)).collect::<Vec<_>>(),
            [5, 6, 7, 15, 16]
        );
        assert_eq!(
            (0..5).map(|k| deal_index(a, a, k)).collect::<Vec<_>>(),
            [8, 9, 17, 18, 19]
        );
        assert_eq!(deal_index(d, a, 0), 0, "the dealer's left goes first");
    }

    #[test]
    fn seat_names_follow_d16() {
        let v = view();
        assert_eq!(seat_name(&v, Seat::SOUTH), "You");
        assert_eq!(seat_name(&v, Seat::WEST), "Left opponent");
        assert_eq!(seat_name(&v, Seat::NORTH), "Your partner");
        assert_eq!(seat_name(&v, Seat::EAST), "Right opponent");
        assert_eq!(player_name(&v, Seat::SOUTH), "You");
    }

    #[test]
    fn instruction_strings_are_verbatim() {
        let mut v = view();
        v.status = GameStatus::Abandoned;
        assert_eq!(
            turn_instruction(&v),
            "This game has ended. Start a new game whenever you’re ready."
        );
        v.status = GameStatus::Active;
        v.phase = GamePhase::HandScore;
        assert_eq!(turn_instruction(&v), "Hand complete. Adding the points…");
        v.phase = GamePhase::TrickResolve;
        v.awaiting_trick_review = Some(true);
        v.trick.index = 4;
        assert_eq!(turn_instruction(&v), "Press Continue to finish this hand.");
        v.trick.index = 1;
        v.trick.winner_seat = Some(Seat::SOUTH);
        assert_eq!(turn_instruction(&v), "You lead next. Press Continue first.");
        v.trick.winner_seat = Some(Seat::WEST);
        assert_eq!(
            turn_instruction(&v),
            "Press Continue when you’re ready for the next trick."
        );
        v.awaiting_trick_review = None;
        assert_eq!(turn_instruction(&v), "Left opponent wins the trick.");
        v.phase = GamePhase::TrickPlay;
        v.turn_seat = Some(Seat::NORTH);
        assert_eq!(turn_instruction(&v), "Your partner is taking a turn…");
        v.turn_seat = Some(Seat::SOUTH);
        v.trick.led_suit = Some(Suit::Hearts);
        assert_eq!(
            turn_instruction(&v),
            "Hearts were led. Follow suit if you can."
        );
        v.trick.led_suit = None;
        assert_eq!(
            turn_instruction(&v),
            "You lead. Select any card, then press Play."
        );
    }

    #[test]
    fn illegal_and_unavailable_reasons_keep_their_apostrophes() {
        let mut v = view();
        let card = v.hand[0];
        v.turn_seat = Some(Seat::WEST);
        assert_eq!(illegal_reason(&v, card), "It's not your turn yet.");
        assert_eq!(unavailable_reason(&v, card, false), "Wait for your turn.");
        assert_eq!(
            unavailable_reason(&v, card, true),
            "Please wait until the table is ready."
        );
        v.turn_seat = Some(Seat::SOUTH);
        v.phase = GamePhase::BidRound1;
        assert_eq!(
            illegal_reason(&v, card),
            "Finish bidding before you play a card."
        );
        assert_eq!(
            unavailable_reason(&v, card, false),
            "Finish bidding before selecting a card."
        );
        v.status = GameStatus::Complete;
        assert_eq!(illegal_reason(&v, card), "This game is finished.");
        assert_eq!(
            unavailable_reason(&v, card, false),
            "This match is finished."
        );
    }

    #[test]
    fn bid_labels_come_from_the_move() {
        let v = view();
        let m = |mv: Move| LegalMove {
            id: String::new(),
            mv,
            label: "server".to_owned(),
        };
        assert_eq!(bid_label(&v, &m(Move::Pass)), "Pass");
        assert_eq!(
            bid_label(
                &v,
                &m(Move::Call {
                    suit: Suit::Spades,
                    alone: false
                })
            ),
            "Spades"
        );
        assert_eq!(bid_label(&v, &m(Move::Cut { cut: true })), "Cut the deck");
        assert_eq!(
            bid_label(&v, &m(Move::Cut { cut: false })),
            "Deal the cards"
        );
        let up = v.up_card.expect("an up card");
        assert_eq!(
            bid_label(&v, &m(Move::OrderUp { alone: false })),
            format!("Make {} trump", suit_name(up.printed_suit()).to_lowercase())
        );
        assert!(bid_is_secondary(&m(Move::Pass)));
        assert!(bid_is_secondary(&m(Move::Cut { cut: false })));
    }

    #[test]
    fn result_line_priority() {
        let mut v = view();
        v.phase = GamePhase::TrickPlay;
        v.trick.index = 2;
        assert_eq!(
            trick_result(&v, None),
            ResultLine::Plain("Trick 3 of 5 · The winner leads the next trick.".to_owned())
        );
        assert!(matches!(
            trick_result(&v, Some((Seat::WEST, "Hi"))),
            ResultLine::Talk { ref quote, .. } if quote == "“Hi”"
        ));
        v.trick.winner_seat = Some(Seat::SOUTH);
        assert_eq!(
            trick_result(&v, None),
            ResultLine::Won {
                strong: "You win the trick.".to_owned(),
                next: "You lead next.".to_owned()
            }
        );
        v.review_tricks = Some(true);
        v.trick.winner_seat = Some(Seat::EAST);
        assert_eq!(
            trick_result(&v, None),
            ResultLine::Won {
                strong: "Right opponent wins the trick.".to_owned(),
                next: "Press Continue before the next lead.".to_owned()
            }
        );
    }

    #[test]
    fn play_button_and_score_labels() {
        let v = view();
        assert_eq!(play_button_label(&v, None, false), "Play selected card →");
        assert_eq!(
            score_cards_label(7),
            "7 points, counted with a six and a four"
        );
    }
}
