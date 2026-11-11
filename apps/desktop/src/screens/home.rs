//! Home (06 §3): the header, the hero, the introduction, how to play and the footer.

use rune::{Color, Cursor, Cx, El, ScrollSpec, Span, SurfaceFit, col, el, row, text};

use crate::app::{AppState, NAV_LINKS, Route, nav_link};
use crate::screens::wordmark;
use crate::theme::{PAL, home as h, motion, radius, space, ty};

/// Home's buttons shrink to this while held (09 §5.4 key-button physics).
const BUTTON_PRESS: f32 = 0.96;

/// Eyebrow over the hero headline.
pub const EYEBROW: &str = "THE TABLE IS YOURS";
/// The headline, first line.
pub const H1_LINE_1: &str = "A little nerve.";
/// The headline, second line (its last word is lime).
pub const H1_LINE_2: &str = "A great hand.";
/// The hero lede.
pub const LEDE: &str = "Five cards. Four seats. Endless good calls.\nThe classic card game, with a fresh seat at the table.";
/// The primary action.
pub const PLAY: &str = "Let’s play";
/// The secondary action.
pub const NEW_TO_EUCHRE: &str = "New to euchre? ↓";
/// The note under the actions.
pub const NOTE: &str = "Play against computer opponents · No account needed";
/// The hero art's alt text.
pub const HERO_ALT: &str = "Classic playing cards and a dealer button on a green felt table, with both red jacks at the front";
/// The caption under the art.
pub const CAPTION: &str = "GOOD COMPANY. BETTER CARDS.";
/// The bottom strip.
pub const STRIP: [&str; 3] = [
    "A CLASSIC, DEALT DIFFERENTLY.",
    "♠ ♥ ♣ ♦",
    "FIRST TO TEN. MAKE IT COUNT.",
];
/// The bower stamp.
pub const STAMP: [&str; 3] = ["THE RIGHT", "J♥", "KIND OF LUCK"];
/// The header caption next to the wordmark.
pub const BRAND_CAPTION: &str = "THE CARD CLUB";

/// Introduction eyebrow.
pub const INTRO_EYEBROW: &str = "SMALL DECK. BIG PERSONALITY.";
/// Introduction heading.
pub const INTRO_H2: &str = "All the feeling.\nNone of the shuffling.";
/// Introduction paragraph 1 (06 §3.2: "made for the desktop").
pub const INTRO_P1: &str = "A proper game of partnership euchre, made for the desktop. Read the table, trust your partner, and know when to go it alone.";
/// Introduction paragraph 2.
pub const INTRO_P2: &str =
    "Settle into a 3D table with three computer players. Your next good hand is one deal away.";

/// How to play eyebrow.
pub const HOW_EYEBROW: &str = "PULL UP A CHAIR";
/// How to play heading, first line.
pub const HOW_H2_1: &str = "Easy to learn.";
/// How to play heading, second line (italic).
pub const HOW_H2_2: &str = "Hard to put down.";
/// The three rule cards: number, suit glyph, title, body.
pub const RULES: [(&str, &str, &str, &str); 3] = [
    (
        "01 /",
        "♣",
        "Find your partner.",
        "Four players. Two teams. The player across the table is on your side. You each get five cards.",
    ),
    (
        "02 /",
        "♦",
        "Make the call.",
        "Choose a trump suit or pass. The team that calls trump needs to take at least three of the five tricks.",
    ),
    (
        "03 /",
        "♠",
        "Play your hand.",
        "Follow suit if you can. The highest card wins the trick, and trump beats everything else. First team to ten wins.",
    ),
];
/// The disclosure.
pub const JACKS: &str = "The jacks have a little secret";
/// Disclosure paragraph 1 (the bowers are bold).
pub const JACKS_P1: &str = "The jack of trump is the right bower, the strongest card. The other jack of the same color is the left bower, the second strongest. Both belong to the trump suit.";
/// Disclosure paragraph 2.
pub const JACKS_P2: &str = "Take three or four tricks after calling trump for one point, or all five for two. Get stopped and the other team earns two. Feeling bold? Go alone and take all five for four points.";

/// Footer tagline.
pub const FOOTER_TAG: &str = "A good hand is better shared.";
/// Footer link.
pub const FOOTER_LINK: &str = "See you at the table ↗";

/// The Home screen at window width `width`.
pub fn view(state: &AppState, width: f32, cx: &mut Cx<AppState>) -> El {
    let narrow = width <= crate::theme::window::NARROW_W;
    let gutter = if narrow {
        h::GUTTER_NARROW
    } else {
        space::HOME_GUTTER.max((width - space::HOME_MAX_W) / 2.0)
    };
    let page = col()
        .key("home-page")
        .w_full()
        .px(gutter)
        .child(header(&state.route, narrow))
        .child(hero(state, width - 2.0 * gutter, narrow, cx))
        .child(intro(width, narrow))
        .child(how_to_play(state, width, narrow))
        .child(footer(narrow));
    col()
        .key("home")
        .w_full()
        .h_full()
        .fill(PAL.cream)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

fn header(here: &Route, narrow: bool) -> El {
    let brand = row()
        .key("home-brand")
        .gap(space::SM)
        .center_y()
        .cursor(Cursor::Pointer)
        .role("link")
        .label("Euchre home")
        .child(wordmark(
            "home-wordmark",
            ty::BRAND,
            ty::BRAND_CLUB,
            PAL.ink,
        ));
    let brand = if narrow {
        brand
    } else {
        brand.child(
            text(BRAND_CAPTION)
                .key("home-brand-caption")
                .font(ty::BRAND_CAPTION)
                .color(PAL.text_secondary)
                .wrap()
                .max_w(h::BRAND_CAPTION_W)
                .pl(space::S)
                .border_sides([0.0, 0.0, 0.0, 1.0], PAL.brand_caption_border),
        )
    };
    let mut links = row()
        .key("home-nav")
        .role("navigation")
        .label("Main navigation")
        .gap(space::XS)
        .center_y();
    for (label, target, title) in NAV_LINKS {
        links = links.child(nav_link(label, target, title, here, true));
    }
    row()
        .key("home-header")
        .w_full()
        .h(if narrow {
            h::HEADER_H_NARROW
        } else {
            space::HOME_HEADER_H
        })
        .px(if narrow {
            h::HEADER_INSET_NARROW
        } else {
            h::HEADER_INSET
        })
        .center_y()
        .justify_between()
        .child(brand)
        .child(links)
}

fn hero(state: &AppState, hero_w: f32, narrow: bool, cx: &mut Cx<AppState>) -> El {
    let pad_x = hero_w * space::HERO_PX_FRAC;
    let eyebrow = row()
        .key("hero-eyebrow")
        .gap(space::SM)
        .center_y()
        .child(
            el().size(space::EYEBROW_DOT, space::EYEBROW_DOT)
                .radius(radius::PILL)
                .fill(PAL.lime)
                .box_shadow(rune::Shadow {
                    spread: space::EYEBROW_HALO,
                    color: PAL.lime_halo,
                    ..rune::Shadow::default()
                }),
        )
        .child(text(EYEBROW).font(ty::EYEBROW).color(PAL.hero_eyebrow));
    let headline = format!("{H1_LINE_1}\n{H1_LINE_2}");
    let last_word = headline.rfind("hand.").unwrap_or(headline.len());
    let h1 = text(headline.clone())
        .key("hero-h1")
        .font(ty::home_h1(hero_w + 2.0 * space::HOME_GUTTER))
        .color(PAL.hero_text)
        .role("heading")
        .level(1)
        .spans(vec![
            Span::new(last_word, headline.len())
                .color(PAL.lime)
                .weight(400.0),
        ]);
    let lede = text(LEDE)
        .key("hero-lede")
        .font(ty::HERO_LEDE)
        .color(PAL.hero_lede)
        .wrap()
        .max_w(h::LEDE_MAX_W);
    let play = row()
        .key("hero-play")
        .gap(space::HERO_BUTTON_GAP)
        .center_y()
        .px(space::HERO_BUTTON_PX)
        .py(space::HERO_BUTTON_PY)
        .radius(radius::HERO_BUTTON)
        .fill(PAL.lime)
        .hover(PAL.lime_hover)
        // Key-button physics (09 §5.4, §5.7): the hover springs in and out, a press dips.
        .hover_motion(motion::HOVER, motion::HOVER)
        .press_scale(BUTTON_PRESS)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("link")
        .label(PLAY)
        .on_press_tx(|s: &mut AppState, tx| s.navigate(Route::NewGame, tx))
        .child(text(PLAY).font(ty::HERO_BUTTON).color(PAL.lime_text))
        .child(text("↗").font(ty::HERO_BUTTON).color(PAL.lime_text));
    let learn = text(NEW_TO_EUCHRE)
        .key("hero-learn")
        .font(ty::HERO_LINK)
        .color(PAL.hero_text)
        .spans(vec![
            Span::new(0, NEW_TO_EUCHRE.len()).underline(PAL.hero_text),
        ])
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("link")
        .on_press_tx(|_s: &mut AppState, tx| tx.scroll_into_view("how-to-play"));
    let actions = row()
        .key("hero-actions")
        .gap(space::L)
        .center_y()
        .flex_wrap()
        .child(play)
        .child(learn);
    let copy = col()
        .key("hero-copy")
        .justify_center()
        .gap(space::L)
        .child(eyebrow)
        .child(h1)
        .child(lede)
        .child(actions.mt(space::S))
        .child(
            text(NOTE)
                .key("hero-note")
                .font(ty::HERO_NOTE)
                .color(PAL.hero_note),
        );

    let art = hero_art(state, narrow, cx);
    let body = if narrow {
        col()
            .key("hero-body")
            .w_full()
            .gap(space::L)
            .child(copy.w_full())
            .child(art)
    } else {
        row()
            .key("hero-body")
            .w_full()
            .grow(1.0)
            .gap(space::L)
            .child(copy.grow(h::HERO_COPY_FR).w(0.0))
            .child(art)
    };

    let strip = strip(narrow);
    col()
        .key("hero")
        .w_full()
        .min_h(if narrow {
            h::HERO_MIN_H_NARROW
        } else {
            space::HERO_MIN_H
        })
        .radius(radius::HERO)
        .fill(PAL.forest)
        .clip_rounded(radius::HERO)
        .pt(space::HERO_PT)
        .pb(space::HERO_PB)
        .px(pad_x)
        .child(body)
        .child(strip.pin(
            Some(pad_x),
            None,
            Some(pad_x),
            Some(space::HERO_STRIP_BOTTOM),
        ))
}

fn hero_art(state: &AppState, narrow: bool, cx: &mut Cx<AppState>) -> El {
    let [col_w, col_h] = cx.layout_size("hero-art").unwrap_or(h::ART_FALLBACK);
    let img_w = col_w * h::ART_SCALE;
    let img_h = img_w / h::ART_ASPECT;
    let (left, top) = ((col_w - img_w) / 2.0, (col_h - img_h) / 2.0);
    let art = el().key("hero-art").label(HERO_ALT).role("img");
    let mut art = if narrow {
        art.w_full().h(h::ART_H_NARROW)
    } else {
        art.grow(1.0).w(0.0).h_full()
    };
    if let Some(a) = &state.art {
        art = art.child(
            el().key("hero-image")
                .size(img_w, img_h)
                .absolute(left, top)
                .pixels_fit(a.hero().pixels(), SurfaceFit::Fit),
        );
    }
    // The radial alpha mask (06 §3.1) as a forest vignette over the picture.
    art = art.child(
        el().key("hero-mask")
            .size(img_w, img_h)
            .absolute(left, top)
            .radial_gradient(
                [0.5, 0.5],
                [img_w * h::MASK_R[0], img_h * h::MASK_R[1]],
                [
                    (h::MASK_SOLID, Color::TRANSPARENT),
                    (h::MASK_CLEAR, PAL.forest),
                    (1.0, PAL.forest),
                ],
            ),
    );
    if !narrow {
        art = art.child(
            text(CAPTION)
                .key("hero-caption")
                .font(ty::HERO_CAPTION)
                .color(PAL.hero_caption)
                .pin(
                    None,
                    None,
                    Some(h::CAPTION_INSET[0]),
                    Some(h::CAPTION_INSET[1]),
                ),
        );
    }
    art.child(stamp().pin(None, None, Some(h::STAMP_INSET[0]), Some(h::STAMP_INSET[1])))
}

fn stamp() -> El {
    let j = STAMP[1];
    let heart = j.find('♥').unwrap_or(j.len());
    col()
        .key("hero-stamp")
        .size(space::STAMP, space::STAMP)
        .center()
        .radius(radius::PILL)
        .fill(PAL.lime)
        .border(space::STAMP_RING, PAL.forest)
        .rotate(h::STAMP_TURN)
        .a11y_hidden()
        .child(text(STAMP[0]).font(ty::STAMP_LINE).color(PAL.lime_text))
        .child(text(j).font(ty::STAMP_J).color(PAL.lime_text).spans(vec![
                Span::new(heart, j.len())
                    .size(ty::STAMP_HEART)
                    .color(PAL.suit_red),
            ]))
        .child(text(STAMP[2]).font(ty::STAMP_LINE).color(PAL.lime_text))
}

fn strip(narrow: bool) -> El {
    let suits = STRIP[1];
    let mut spans = Vec::new();
    for red in ['♥', '♦'] {
        if let Some(at) = suits.find(red) {
            spans.push(Span::new(at, at + red.len_utf8()).color(PAL.hero_suit_red));
        }
    }
    let mut row = row()
        .key("hero-strip")
        .pt(space::M)
        .border_sides([1.0, 0.0, 0.0, 0.0], PAL.hero_divider)
        .center_y()
        .justify_between()
        .child(text(STRIP[0]).font(ty::HERO_STRIP).color(PAL.hero_muted))
        .child(
            text(suits)
                .font(ty::HERO_SUITS)
                .color(PAL.hero_muted)
                .spans(spans)
                .a11y_hidden(),
        );
    if !narrow {
        row = row.child(text(STRIP[2]).font(ty::HERO_STRIP).color(PAL.hero_muted));
    }
    row
}

fn section(key: &str) -> El {
    col()
        .key(key.to_owned())
        .w_full()
        .max_w(space::INTRO_MAX_W)
        .self_center()
        .px(space::L)
        .py(space::INTRO_PY)
}

fn eyebrow(key: &str, label: &str) -> El {
    text(label.to_owned())
        .key(key.to_owned())
        .font(ty::EYEBROW)
        .color(PAL.text_secondary)
}

fn intro(width: f32, narrow: bool) -> El {
    let left = col()
        .key("intro-heading")
        .gap(space::M)
        .child(eyebrow("intro-eyebrow", INTRO_EYEBROW))
        .child(
            text(INTRO_H2)
                .key("intro-h2")
                .font(ty::section_h2(width))
                .color(PAL.ink)
                .role("heading")
                .level(2),
        );
    let right = col()
        .key("intro-copy")
        .gap(space::M)
        .child(text(INTRO_P1).font(ty::INTRO_P).color(PAL.ink).wrap())
        .child(
            text(INTRO_P2)
                .font(ty::INTRO_P_MUTED)
                .color(PAL.text_muted)
                .wrap(),
        );
    let body = if narrow {
        col().gap(space::L).child(left).child(right)
    } else {
        row()
            .gap(space::INTRO_GAP)
            .child(left.grow(1.0).w(0.0))
            .child(right.grow(1.0).w(0.0).pt(space::L))
    };
    section("intro").child(body)
}

fn rule_card(i: usize, (number, suit, title, body): (&str, &str, &str, &str)) -> El {
    let suit_color = if suit == "♦" { PAL.suit_red } else { PAL.ink };
    col()
        .key(format!("rule-{i}"))
        .gap(space::SM)
        .padding(space::RULE_CARD_PAD)
        .radius(radius::RULE_CARD)
        .fill(PAL.rule_card)
        .child(
            row()
                .justify_between()
                .items_start()
                .child(text(number).font(ty::RULE_NUMBER).color(PAL.rule_number))
                .child(
                    text(suit)
                        .font(ty::RULE_SUIT)
                        .color(suit_color)
                        .a11y_hidden(),
                ),
        )
        .child(
            text(title)
                .font(ty::RULE_H3)
                .color(PAL.ink)
                .role("heading")
                .level(3)
                .mt(h::RULE_TITLE_GAP),
        )
        .child(
            text(body)
                .font(ty::RULE_BODY)
                .color(PAL.text_secondary)
                .wrap(),
        )
}

fn bold_bowers(line: &str) -> Vec<Span> {
    ["right bower", "left bower"]
        .iter()
        .filter_map(|w| {
            line.find(w)
                .map(|at| Span::new(at, at + w.len()).weight(h::STRONG))
        })
        .collect()
}

fn how_to_play(state: &AppState, width: f32, narrow: bool) -> El {
    let heading = format!("{HOW_H2_1}\n{HOW_H2_2}");
    let at = heading.len() - HOW_H2_2.len();
    let mut cards = if narrow { col() } else { row() }
        .key("rules")
        .gap(space::M);
    for (i, rule) in RULES.into_iter().enumerate() {
        let card = rule_card(i, rule);
        // In a column `grow` acts vertically, so a zero basis width would
        // collapse the card: narrow windows take the full width instead.
        cards = cards.child(if narrow {
            card.w_full()
        } else {
            card.grow(1.0).w(0.0)
        });
    }
    let open = state.jacks_open;
    let disclosure = row()
        .key("jacks")
        .min_h(space::TAP)
        .gap(space::SM)
        .center_y()
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("button")
        .label(JACKS)
        .expanded(open)
        .on_press(|s: &mut AppState| s.jacks_open = !s.jacks_open)
        .child(text(JACKS).font(ty::DISCLOSURE).color(PAL.ink))
        .child(
            text("+")
                .key("jacks-plus")
                .font(ty::DISCLOSURE)
                .color(PAL.ink)
                .motion(motion::UI)
                .rotate(if open { h::PLUS_TURN } else { 0.0 }),
        );
    let mut s = section("how-to-play")
        .gap(space::L)
        .child(eyebrow("how-eyebrow", HOW_EYEBROW))
        .child(
            text(heading.clone())
                .key("how-h2")
                .font(ty::section_h2(width))
                .color(PAL.ink)
                .role("heading")
                .level(2)
                .spans(vec![Span::new(at, heading.len()).italic()]),
        )
        .child(cards)
        .child(disclosure);
    if open {
        let p = |key: &str, line: &str| {
            text(line.to_owned())
                .key(key.to_owned())
                .font(ty::RULE_BODY)
                .color(PAL.ink)
                .wrap()
                .spans(bold_bowers(line))
        };
        let both = if narrow {
            col()
                .key("jacks-body")
                .w_full()
                .gap(space::L)
                .child(p("jacks-p1", JACKS_P1).w_full())
                .child(p("jacks-p2", JACKS_P2).w_full())
        } else {
            row()
                .key("jacks-body")
                .gap(space::XL)
                .child(p("jacks-p1", JACKS_P1).grow(1.0).w(0.0))
                .child(p("jacks-p2", JACKS_P2).grow(1.0).w(0.0))
        };
        s = s.child(both);
    }
    s
}

fn footer(narrow: bool) -> El {
    let mut f = row()
        .key("home-footer")
        .w_full()
        .py(space::XL)
        .border_sides([1.0, 0.0, 0.0, 0.0], PAL.hairline)
        .center_y()
        .justify_between()
        .child(wordmark(
            "footer-wordmark",
            ty::BRAND_FOOTER,
            ty::BRAND_FOOTER_CLUB,
            PAL.ink,
        ));
    if !narrow {
        f = f.child(text(FOOTER_TAG).font(ty::FOOTER_TAG).color(PAL.text_muted));
    }
    // A text box keeps its glyphs at the top of a taller box, so the tap
    // target is a row that centres the link vertically.
    f.child(
        row()
            .key("footer-play")
            .min_h(space::TAP)
            .center_y()
            .press_scale(BUTTON_PRESS)
            .cursor(Cursor::Pointer)
            .focusable(true)
            .role("link")
            .label(FOOTER_LINK)
            .on_press_tx(|s: &mut AppState, tx| s.navigate(Route::NewGame, tx))
            .child(
                text(FOOTER_LINK)
                    .font(ty::FOOTER_LINK)
                    .color(PAL.ink)
                    .a11y_hidden(),
            ),
    )
}
