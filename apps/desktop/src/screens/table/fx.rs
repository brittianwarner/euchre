//! The effects layer (09 §4.3, §4.8, §4.9): the play flyers, the stamp sparks, the euchre and
//! march stamps, and the confetti. table/mod.rs mounts [`layer`] inside a pinned, raised,
//! accessibility-hidden `fx-layer` box over the whole table (after the body, before the
//! settings layer and the dialogs, so dialogs still cover it).
//!
//! Everything here is posed from `AppState.clock` and the [`super::choreo::Director`], so a
//! frame is a pure function of the state (09 §2.1). Nothing is built under reduce motion. The
//! box is the window, so a window point is a point in this layer. It takes no pointer input of
//! its own; keep it that way (no handlers here), so clicks reach the table under it.
//!
//! Paint order is tree order: flyers, then the sparks, then the stamp over them, then the
//! confetti last so it falls over everything.

use std::f32::consts::PI;

use euchre_engine::team_of;
use euchre_protocol::{HandResult, PublicGameView};
use rune::{Color, Cx, El, Sequence, Shape, Transition, canvas, el, row, text};

use super::choreo::{self, Feel, ShotKey, play_pose};
use super::stage::{self, hash};
use crate::app::AppState;
use crate::theme::felt::{size, ty as fty};
use crate::theme::{self, FELT, motion};

/// What the effects layer draws this commit.
pub fn layer(ctx: &stage::Ctx<'_>, cx: &Cx<AppState>) -> El {
    let mut fx = el()
        .key("fx")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if ctx.reduced {
        return fx;
    }
    for flyer in flyers(ctx, cx) {
        fx = fx.child(flyer);
    }
    if let Some((sparks, stamp)) = stamp(ctx, cx) {
        if let Some(s) = sparks {
            fx = fx.child(s);
        }
        fx = fx.child(stamp);
    }
    if let Some(c) = confetti(ctx, cx) {
        fx = fx.child(c);
    }
    fx
}

/// The window's size in points, from the fx layer's last layout.
fn window(cx: &Cx<AppState>) -> [f32; 2] {
    cx.layout_size("fx-layer")
        .or_else(|| cx.layout_size("root"))
        .unwrap_or([theme::window::SIZE.0, theme::window::SIZE.1])
}

/// The Full-tier play flyers (09 §4.3, mechanism A): each card posed from its flight every
/// frame, from its source (a fan's top back or a rack card) to its trick slot. The slot stays
/// empty until the flyer's `PLAY_END`, then the static card takes over with no cross-fade.
fn flyers(ctx: &stage::Ctx<'_>, cx: &Cx<AppState>) -> Vec<El> {
    let director = &ctx.present().director;
    director
        .flyers(ctx.state.clock)
        .filter_map(|(key, spec, t)| {
            let f = choreo::resolve_flight(spec, ctx.view, cx, ctx.height)?;
            let p = play_pose(&f, t);
            let [w, h] = f.size;
            Some(
                motion::card_shadows(stage::face(ctx.state, spec.card, w), p.lift, 0.0)
                    .key(format!("fly-{}", key.as_str()))
                    .absolute(p.x - w / 2.0, p.y - h / 2.0)
                    .rotate(p.rot)
                    .scale(p.scale)
                    .origin(0.5, 0.5)
                    .instant_changes()
                    .transition(Transition::none()),
            )
        })
        .collect()
}

/// The stamp's word for a hand result (06 §9 copy), if it gets one.
fn stamp_word(result: HandResult) -> Option<&'static str> {
    match result {
        HandResult::Euchre => Some("Euchre!"),
        HandResult::March => Some("March!"),
        HandResult::LoneMarch => Some("Lone march!"),
        _ => None,
    }
}

/// Whether the hand went your team's way: you euchred them, or your team marched.
fn yours(view: &PublicGameView, result: HandResult) -> bool {
    let Some(maker) = view.maker_seat else {
        return false;
    };
    let ours = team_of(maker) == team_of(view.you);
    match result {
        HandResult::Euchre => !ours,
        HandResult::March | HandResult::LoneMarch => ours,
        _ => false,
    }
}

/// Whether the hand's stamp shows this commit. The hand-result card hides its title under it,
/// since both say the same word, and shows it again as the stamp fades.
pub(super) fn stamp_live(ctx: &stage::Ctx<'_>) -> bool {
    let director = &ctx.present().director;
    let key = ShotKey::stamp(ctx.view.hand_no);
    !ctx.reduced
        && director.elapsed(&key, ctx.state.clock).is_some()
        && director.feel_of(&key).is_some_and(|f| f != Feel::Snap)
        && ctx.view.result.and_then(stamp_word).is_some()
}

/// Where the stamp sits: over the hand-result card's title (hidden while the stamp shows),
/// with its lower edge clear of the card's line, so "You score 2." stays readable under it.
/// With no card to sit on, it is centred across the trick area's upper part. Window points.
/// `None` for the one commit before a mounting card is laid out, so the stamp never jumps.
fn stamp_centre(ctx: &stage::Ctx<'_>, cx: &Cx<AppState>) -> Option<[f32; 2]> {
    let hand = ctx.view.hand_no;
    // Half the stamp's height: a 50 pt line, its padding and border.
    let half = (50.0 + 2.0 * (6.0 + size::STAMP[1])) / 2.0;
    if let Some([x, y, w, _]) = cx.layout(&format!("hand-result-{hand}-line")) {
        // The turn drops the plate about 3 pt over the line's width; 2 pt of air under that.
        return Some([x + w / 2.0, y - half - 5.0]);
    }
    let card_mounts = !ctx.present().director.pending(&ShotKey::score(hand))
        && super::copy::hand_result(ctx.view).is_some();
    if card_mounts {
        return None;
    }
    let [x, y, w, h] = cx.layout("trick")?;
    Some([x + w / 2.0, y + (h * 0.30).max(half + 10.0)])
}

/// The euchre, march or lone-march stamp (09 §4.8) and, when the hand went your way, the
/// sparks behind it (09 §4.9). It lands on `STAMP` while the hand's `Stamp` shot runs (a
/// snapped shot shows none), holds, and leaves by opacity when the shot ends.
fn stamp(ctx: &stage::Ctx<'_>, cx: &Cx<AppState>) -> Option<(Option<El>, El)> {
    let view = ctx.view;
    let director = &ctx.present().director;
    let key = ShotKey::stamp(view.hand_no);
    let t = director.elapsed(&key, ctx.state.clock)?;
    let feel = director.feel_of(&key)?;
    if feel == Feel::Snap {
        return None;
    }
    let result = view.result?;
    let word = stamp_word(result)?;
    let [x, y] = stamp_centre(ctx, cx)?;
    let sparks =
        (yours(view, result) && matches!(feel, Feel::Full | Feel::Brisk) && t < motion::SPARKS)
            .then(|| {
                canvas(sparks(t, x, y))
                    .key(format!("sparks-{}", view.hand_no))
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .instant_changes()
                    .transition(Transition::none())
                    .a11y_hidden()
            });
    let [_, border, radius, pad] = size::STAMP;
    // A fine inner line inside the gold border reads as an inked stamp, not a button.
    let inner = el()
        .pin(Some(3.0), Some(3.0), Some(3.0), Some(3.0))
        .radius(radius - 3.0)
        .border(1.0, FELT.stamp_ink.alpha(0.35))
        .a11y_hidden();
    let plate = motion::e5(
        row()
            .key(format!("stamp-{}", view.hand_no))
            .px(pad)
            .py(6.0)
            .radius(radius)
            .fill(FELT.banner)
            .border(border, theme::color(motion::GOLD_DEEP))
            .rotate(size::STAMP_TURN)
            .origin(0.5, 0.5)
            .sequence(Sequence::new(motion::STAMP))
            .transition(Transition::opacity())
            .child(inner)
            .child(text(word).font(fty::STAMP).color(FELT.stamp_ink)),
    );
    // A zero-size anchor at the centre keeps the stamp centred whatever its width.
    let anchor = row()
        .key("stamp-anchor")
        .absolute(x, y)
        .size(0.0, 0.0)
        .center()
        .child(plate);
    Some((sparks, anchor))
}

/// 28 gold and white streaks thrown out from `(x, y)` (09 §4.9, the spike's `sparks`): drag
/// `exp(−4t)`, gravity 160, gone at `SPARKS`. Each starts clear of the stamp's edge. The
/// burst is flat on top (`size::SPARK_RISE`): the partner's plate sits just above the stamp,
/// and no streak crosses its name.
fn sparks(t: f32, x: f32, y: f32) -> Shape {
    let mut shape = Shape::new();
    let n = size::SPARKS;
    let life = (1.0 - t / motion::SPARKS).clamp(0.0, 1.0);
    if life <= 0.0 {
        return shape;
    }
    let drag = (-4.0 * t).exp();
    let fall = 160.0 * t * t;
    for i in 0..n {
        let a = i as f32 / n as f32 * 2.0 * PI + (i as f32 * 1.7).sin() * 0.12;
        let speed = 260.0 + 140.0 * ((i * 7919 % 13) as f32 / 13.0);
        let d = speed * (1.0 - drag) / 4.0;
        let (dx, dy) = (a.cos(), a.sin());
        let (start, ky) = if dy < 0.0 {
            size::SPARK_RISE
        } else {
            (40.0, 1.0)
        };
        let head = [x + dx * (100.0 + d), y + dy * (start + d * ky) + fall];
        // The streak points the way it travels.
        let (ux, uy) = (dx, dy * ky);
        let norm = ux.hypot(uy).max(1e-3);
        let tail_len = 26.0 * drag + 2.0;
        let tail = [
            head[0] - ux / norm * tail_len,
            head[1] - uy / norm * tail_len,
        ];
        let colour = FELT.spark[usize::from(i % 3 == 0)];
        shape = shape.stroke(vec![tail, head], 3.0 * life + 0.5, colour.alpha(life));
    }
    shape
}

/// The confetti (06 §6.5, 09 §4.9). While a happy banner's burst runs
/// (`Presentation::confetti_t`, 3.7 s from the banner's first poll), 120 quads fly from two
/// bottom cannons on a canvas posed from the clock. The match's burst is the match-end
/// panel's ([`fountain`], drawn behind the panel by table/mod.rs). On your team's March
/// (S-4), which has no banner, a renderer-owned fall drops instead, since no frames run for it.
fn confetti(ctx: &stage::Ctx<'_>, cx: &Cx<AppState>) -> Option<El> {
    let [w, h] = window(cx);
    match ctx.present().confetti_t(ctx.state.clock) {
        Some((_, true)) => return None,
        Some((t, false)) => return Some(confetti_canvas(cannons(t, w, h))),
        None => {}
    }
    let view = ctx.view;
    let march = view.result == Some(HandResult::March) && yours(view, HandResult::March);
    if !march {
        return None;
    }
    let key = ShotKey::stamp(view.hand_no);
    let director = &ctx.present().director;
    let live = director
        .elapsed(&key, ctx.state.clock)
        .is_some_and(|_| director.feel_of(&key) != Some(Feel::Snap));
    live.then(|| stage::confetti_fall(format!("confetti-march-{}", view.hand_no), [w, h]))
}

/// A full-window confetti canvas over `shape`, posed from the clock.
pub(super) fn confetti_canvas(shape: Shape) -> El {
    canvas(shape)
        .key("confetti-canvas")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .instant_changes()
        .transition(Transition::none())
        .a11y_hidden()
}

/// The confetti burst `t` seconds in over a `w` by `h` window (the spike's two cannons):
/// pieces leave both bottom corners at 760-1180 pt/s, 60-82 degrees up, slow on drag 1.6,
/// flutter and tumble (the width squashes with `|cos|`), and fade over their last 0.5 s.
fn cannons(t: f32, w: f32, h: f32) -> Shape {
    let mut shape = Shape::new();
    for i in 0..size::CONFETTI_QUADS {
        let left = i % 2 == 0;
        let (ox, dir) = if left {
            (0.06 * w, 1.0)
        } else {
            (0.94 * w, -1.0)
        };
        let ang = (60.0 + 22.0 * hash(i * 13 + 3)).to_radians();
        shape = piece(shape, i, t, [ox, h * 0.95], [dir * ang.cos(), -ang.sin()]);
    }
    shape
}

/// The match's confetti (09 §4.10) `t` seconds in: pieces leave from behind the match-end
/// panel's top edge, `[left, right, top]` in window points, and rise past the top of the
/// window, fanned up to 55 degrees either side of straight up (the outer pieces lean
/// outward), before they drift down over the table. The panel covers the start of every
/// flight, so nothing is seen to pop out of nowhere.
pub(super) fn fountain(t: f32, [left, right, top]: [f32; 3]) -> Shape {
    let mut shape = Shape::new();
    let n = size::CONFETTI_QUADS;
    for i in 0..n {
        let u = (i as f32 + 0.5) / n as f32;
        let across = hash(i * 19 + 7);
        let ox = left + (right - left) * (0.1 + 0.8 * across);
        // Lean with the side the piece starts on, with a little spread either way.
        let lean = (across - 0.5) * 1.6 + (hash(i * 23 + 9) - 0.5) * 0.5;
        let ang = (lean.clamp(-1.0, 1.0) * 55.0).to_radians();
        let at = [ox, top + 14.0 + 6.0 * u];
        shape = piece(shape, i, t, at, [ang.sin(), -ang.cos()]);
    }
    shape
}

/// One tumbling confetti quad `t` seconds into its burst: it leaves `at` along `dir` (a unit
/// vector) at 760-1180 pt/s, slows on drag 1.6, falls, flutters and tumbles (the width
/// squashes with `|cos|`), and fades over its last 0.5 s.
fn piece(shape: Shape, i: u32, t: f32, at: [f32; 2], dir: [f32; 2]) -> Shape {
    let [pw, ph] = [size::CONFETTI[0] + 1.0, size::CONFETTI[1] * 0.36];
    let (life0, life1) = motion::CONFETTI_LIFE;
    let life = life0 + (life1 - life0) * hash(i * 7 + 1);
    let tt = (t - hash(i * 3 + 5) * 0.25).max(0.0);
    if tt > life {
        return shape;
    }
    let speed = 760.0 + 420.0 * hash(i * 11 + 2);
    let (vx, vy) = (dir[0] * speed, dir[1] * speed);
    let drag = (-1.6 * tt).exp();
    let x = at[0] + vx * (1.0 - drag) / 1.6 + 30.0 * (tt * 3.0 + i as f32).sin() * (1.0 - drag);
    let y = at[1] + vy * (1.0 - drag) / 1.6 + 420.0 * tt * tt * 0.5 * 0.55 + 90.0 * tt;
    let spin = tt * (4.0 + 6.0 * hash(i * 17 + 4)) + i as f32;
    let (sw, sh) = (pw * spin.cos().abs().max(0.15), ph);
    let (cs, sn) = (spin.sin(), spin.cos());
    let pts: Vec<[f32; 2]> = [[-sw, -sh], [sw, -sh], [sw, sh], [-sw, sh]]
        .iter()
        .map(|p| [x + p[0] * sn - p[1] * cs, y + p[0] * cs + p[1] * sn])
        .collect();
    let fade = ((life - tt) / 0.5).clamp(0.0, 1.0);
    let colour: Color = FELT.confetti[i as usize % FELT.confetti.len()];
    shape.fill(pts, colour.alpha(fade))
}
