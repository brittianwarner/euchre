//! `randomFor` (`ai/bid-rollouts.ts`): the deterministic RNG every rollout uses.
//!
//! The seed is an FNV-1a fold over the text, iterated **by code point** (`for
//! (const c of text)`) and hashing only the first UTF-16 unit of each code point
//! (`c.charCodeAt(0)`). That differs from the engine's `fnv1a` for astral
//! characters, so it is ported here rather than reused. The stream itself is the
//! engine's mulberry32.

use euchre_engine::Mulberry32;

/// The FNV-1a offset basis the TS seed starts from.
const FNV_OFFSET: u32 = 2_166_136_261;
/// The FNV-1a prime.
const FNV_PRIME: u32 = 16_777_619;

/// The `randomFor` seed of `text`.
pub fn seed_for(text: &str) -> u32 {
    let mut seed = FNV_OFFSET;
    let mut buf = [0u16; 2];
    for c in text.chars() {
        let first = c.encode_utf16(&mut buf)[0];
        seed = (seed ^ u32::from(first)).wrapping_mul(FNV_PRIME);
    }
    seed
}

/// `randomFor(text)`: a mulberry32 stream seeded from `text`.
pub fn random_for(text: &str) -> Mulberry32 {
    Mulberry32::new(seed_for(text))
}

/// The in-place Fisher–Yates shuffle both rollouts use:
/// `for (i = n-1; i > 0; i--) { j = floor(rng() * (i + 1)); swap(i, j) }`.
pub fn shuffle_in_place<T>(items: &mut [T], rng: &mut Mulberry32) {
    let mut i = items.len();
    while i > 1 {
        i -= 1;
        let j = (rng.next_f64() * (i as f64 + 1.0)).floor() as usize;
        items.swap(i, j.min(i));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_hashes_first_unit_of_each_code_point() {
        // "a" = 0x61: (2166136261 ^ 0x61) * 16777619 mod 2^32.
        assert_eq!(seed_for("a"), 0xe40c_292c);
        // An astral char hashes only its high surrogate (0xD83D for U+1F600).
        assert_eq!(
            seed_for("\u{1F600}"),
            (FNV_OFFSET ^ 0xD83D).wrapping_mul(FNV_PRIME)
        );
    }
}
