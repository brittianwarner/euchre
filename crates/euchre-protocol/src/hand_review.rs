//! The post-hand review contract (`protocol/hand-review.ts`, 02 section 7).
//!
//! A review reveals buried and unplayed cards, so it is built only from a
//! finished hand and served only through `getHandReview` for a hand that has
//! an archive. It never appears in a live view or an AI request.

use euchre_engine::{CardId, HandResult, HandState, Seat, Suit, Trick};
use serde::{Deserialize, Serialize};

use crate::serde_util::double_option;

/// One finished hand, for the review panel (`CompletedHandReview`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedHandReview {
    /// The hand.
    pub hand_no: u32,
    /// Absent in archives written before the review summary existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dealer_seat: Option<Seat>,
    /// Absent in old archives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tricks_won: Option<[u8; 2]>,
    /// `upCard?: CardId | null`: `None` absent, `Some(None)` null.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub up_card: Option<Option<CardId>>,
    /// Trump, or `null` for a throw-in.
    pub trump: Option<Suit>,
    /// The maker, or `null`.
    pub maker_seat: Option<Seat>,
    /// The loner, or `null`.
    pub alone_seat: Option<Seat>,
    /// `hand.trickLog`.
    pub tricks: Vec<Trick>,
    /// Three kitty cards plus the dealer's discard or the turned-down up-card.
    pub buried: Vec<CardId>,
    /// Every card still in a hand, seat 0 to 3.
    pub unplayed: Vec<CardId>,
    /// The result, or `null`.
    pub result: Option<HandResult>,
    /// Points per team, or `null`.
    pub delta: Option<[u8; 2]>,
}

/// Builds the archive for a finished hand exactly as `archiveHand` does.
///
/// `buried = kitty[1..] ++ (dealerDiscard ?? upCard ?? [])`;
/// `unplayed = hands[0] ++ hands[1] ++ hands[2] ++ hands[3]`.
pub fn build_hand_review(hand: &HandState) -> CompletedHandReview {
    let mut buried: Vec<CardId> = hand.kitty[1..].to_vec();
    if let Some(card) = hand.dealer_discard.or(hand.up_card) {
        buried.push(card);
    }
    CompletedHandReview {
        hand_no: hand.hand_no,
        dealer_seat: Some(hand.dealer_seat),
        tricks_won: Some(hand.tricks_won),
        up_card: Some(hand.up_card),
        trump: hand.trump,
        maker_seat: hand.maker_seat,
        alone_seat: hand.alone_seat,
        tricks: hand.trick_log.clone(),
        buried,
        unplayed: hand.hands.iter().flatten().copied().collect(),
        result: hand.result,
        delta: hand.delta,
    }
}
