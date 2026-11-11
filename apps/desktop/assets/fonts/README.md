# Bundled fonts

Both families are under the SIL Open Font License 1.1 (see the `OFL-*.txt` files).

| Files | Family | Source |
|---|---|---|
| `DMSans-{Regular,Medium,SemiBold,Bold}.ttf` | DM Sans | `google/fonts` `ofl/dmsans/DMSans[opsz,wght].ttf`, instanced at opsz 14 and wght 400/500/600/700 with fontTools `varLib.instancer`; the name table was set to family "DM Sans" |
| `SourceSerif4-{Regular,Medium,SemiBold,Bold,Italic}.ttf` | Source Serif 4 | `google/fonts` `ofl/sourceserif4/SourceSerif4[opsz,wght].ttf` (and `-Italic`), instanced at opsz 20 and wght 400/500/600/700 (italic 400) |

Static instances are used because Rune's `AppSpec::fonts` loads faces by weight; the web app relies on browser optical sizing, approximated here by a fixed optical size.

`IBMPlexMono-Regular.ttf` (OFL, copied from Rune's `crates/rune/assets`) fills Rune's mono slot. The web app uses no monospace text, so it appears only in dev overlays.

`NotoSansSymbols2-Suits.ttf` (OFL, `OFL-NotoSansSymbols2.txt`) is Noto Sans Symbols 2 from `google/fonts` `ofl/notosanssymbols2/`, subset with fontTools to U+2660 ♠, U+2663 ♣, U+2665 ♥ and U+2666 ♦ only, so the suit symbols never draw as boxes on the web build or the window.
