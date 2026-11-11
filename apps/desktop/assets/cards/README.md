# Desktop card art

`AS.png` is the Ace of spades from `apps/web/static/art/cards/AS.png` (Adrian Kennard's
traditional faces, CC0 1.0; see `apps/web/static/art/cards/LICENSE.txt`), with the big
decorated spade, its QR code and the `www.me.uk/cards/` line removed (D31, sign-off S-2).

In their place is a plain centre spade drawn the way the other three aces draw theirs: the
`SSA` spade path from `AS.svg`, placed as `<use width="70" height="70" x="-35" y="-35">` on the
`-120 -168 240 336` card, rasterised at 8x and box-filtered down to 512x716. The same method,
run on the club path, reproduces the shipped `AC.png` centre pip to a mean of 0.27/255 per
pixel. The corner indices are the original pixels.

Every other face and the back still come from `apps/web/static/art/cards/`.
