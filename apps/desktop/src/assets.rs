//! Embedded art: the 24 card faces, the card back and the home hero (06 §19), plus credits.
//!
//! The PNGs are embedded from `apps/web/static/art/` so the monorepo keeps one copy. They
//! decode once through [`ImageCache`] (asynchronous, mipmapped: 07 §4).
//!
//! The browser build embeds none of it (3.4 MB of PNG and WebP that brotli cannot shrink): it
//! fetches the same file names by absolute URL under the page's art base (web plan W-8, S2:
//! `crate::web::asset_base()`, e.g. `/_rune/immutable/art-<hash>/cards/9S.png`), where the
//! site build puts the desktop's own Ace of spades over the shared one.

use std::collections::HashMap;

use rune::{Image, ImageCache, Waker};

/// Every card face id, in `CARD_FACES` order: the engine's `Card` display form, and the file
/// stem under `cards/`.
pub const FACE_IDS: [&str; 24] = [
    "9S", "TS", "JS", "QS", "KS", "AS", "9H", "TH", "JH", "QH", "KH", "AH", "9D", "TD", "JD", "QD",
    "KD", "AD", "9C", "TC", "JC", "QC", "KC", "AC",
];

/// Embeds one card face by id, e.g. `face!("9S")`.
#[cfg(not(target_arch = "wasm32"))]
macro_rules! face {
    ($id:literal) => {
        (
            $id,
            include_bytes!(concat!("../../web/static/art/cards/", $id, ".png")).as_slice(),
        )
    };
}

/// The Ace of spades. The shared CC0 face carries a QR code and a URL inside its big spade;
/// the desktop draws its own copy with a plain centre pip the size the other aces use (D31 S-2;
/// how it was made: `assets/cards/README.md`).
#[cfg(not(target_arch = "wasm32"))]
pub const ACE_OF_SPADES: &[u8] = include_bytes!("../assets/cards/AS.png");

/// Every card face as `(card id, PNG bytes)`; ids are the engine's `Card` display form.
#[cfg(not(target_arch = "wasm32"))]
pub const CARD_FACES: [(&str, &[u8]); 24] = [
    face!("9S"),
    face!("TS"),
    face!("JS"),
    face!("QS"),
    face!("KS"),
    ("AS", ACE_OF_SPADES),
    face!("9H"),
    face!("TH"),
    face!("JH"),
    face!("QH"),
    face!("KH"),
    face!("AH"),
    face!("9D"),
    face!("TD"),
    face!("JD"),
    face!("QD"),
    face!("KD"),
    face!("AD"),
    face!("9C"),
    face!("TC"),
    face!("JC"),
    face!("QC"),
    face!("KC"),
    face!("AC"),
];

/// The card back (original art).
#[cfg(not(target_arch = "wasm32"))]
pub const CARD_BACK: &[u8] = include_bytes!("../../web/static/art/cards/back.png");

/// The home hero (1200x1000 WebP).
#[cfg(not(target_arch = "wasm32"))]
pub const HERO: &[u8] = include_bytes!("../../web/static/art/euchre-hero.webp");

/// The card art licence (CC0 1.0 Universal), shown in the credits.
pub const CARD_LICENSE: &str = include_str!("../../web/static/art/cards/LICENSE.txt");

/// Plain-language credits for the About section.
pub const CREDITS: &str = "Card faces: Adrian Kennard's traditional designs (CC0 1.0), packaged by letele/playing-cards. Card back: original art. Type: DM Sans and Source Serif 4, under the SIL Open Font License 1.1.";

/// The decoded art, held in app state for the life of the window.
pub struct Art {
    /// Keeps the decode workers alive.
    _cache: ImageCache,
    faces: HashMap<&'static str, Image>,
    back: Image,
    hero: Image,
}

impl Art {
    /// Starts fetching every image from the page's art base; the browser decodes each off the
    /// main thread and `waker` redraws when one lands.
    #[cfg(target_arch = "wasm32")]
    pub fn load(waker: Waker) -> Art {
        let base = crate::web::asset_base();
        let cache = ImageCache::new(waker);
        let faces = FACE_IDS
            .iter()
            .map(|id| (*id, cache.url(&format!("{base}cards/{id}.png"), 0.0, 1.0)))
            .collect();
        let back = cache.url(&format!("{base}cards/back.png"), 0.0, 1.0);
        let hero = cache.url(&format!("{base}euchre-hero.webp"), 0.0, 1.0);
        Art {
            _cache: cache,
            faces,
            back,
            hero,
        }
    }

    /// Starts decoding every image. `waker` redraws the window when a decode lands.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(waker: Waker) -> Art {
        let cache = ImageCache::new(waker);
        let faces = CARD_FACES
            .iter()
            .map(|(id, bytes)| (*id, cache.load(*bytes)))
            .collect();
        let back = cache.load(CARD_BACK);
        let hero = cache.load(HERO);
        Art {
            _cache: cache,
            faces,
            back,
            hero,
        }
    }

    /// Blocks until every image decoded. Headless renders and tests call this so the first
    /// snapshot already has the pictures (07 §4.3).
    pub fn wait_all(&self) {
        for img in self.faces.values() {
            let _ = img.wait();
        }
        let _ = self.back.wait();
        let _ = self.hero.wait();
    }

    /// The face for a card id such as `"JS"`.
    pub fn face(&self, id: &str) -> Option<&Image> {
        self.faces.get(id)
    }

    /// The card back.
    pub fn back(&self) -> &Image {
        &self.back
    }

    /// The home hero.
    pub fn hero(&self) -> &Image {
        &self.hero
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_face_ids_match_the_embedded_faces() {
        let embedded: Vec<&str> = CARD_FACES.iter().map(|(id, _)| *id).collect();
        assert_eq!(embedded, FACE_IDS, "the browser build fetches these names");
    }

    #[test]
    fn every_engine_card_has_a_face() {
        for card in euchre_engine::CARD_IDS {
            let id = card.to_string();
            assert!(CARD_FACES.iter().any(|(f, _)| *f == id), "{id} has no face");
        }
        assert!(
            CARD_FACES
                .iter()
                .all(|(_, bytes)| bytes.starts_with(b"\x89PNG"))
        );
        assert!(CARD_BACK.starts_with(b"\x89PNG"));
        assert!(CARD_LICENSE.contains("CC0 1.0 Universal"));
    }

    #[test]
    fn the_ace_of_spades_is_the_clean_desktop_copy() {
        let (_, bytes) = CARD_FACES
            .iter()
            .find(|(id, _)| *id == "AS")
            .copied()
            .unwrap_or(("AS", &[]));
        assert_eq!(bytes, ACE_OF_SPADES, "AS uses the desktop copy");
        // Same 512x716 frame as every other face (PNG IHDR width and height).
        let size = |b: &[u8]| (b[16..20].to_vec(), b[20..24].to_vec());
        assert_eq!(size(ACE_OF_SPADES), size(CARD_FACES[0].1));
        // The shared face is much bigger: the QR code and the URL are not in the copy.
        let shared = include_bytes!("../../web/static/art/cards/AS.png");
        assert!(ACE_OF_SPADES.len() * 2 < shared.len());
    }
}
