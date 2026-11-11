# At phone width the fixed nav covers the top of the games pages (W4 review → W5 UI fidelity)

**From:** the W4 integration review (web-flows, P3), checked by the W4 fix agent.
**Owner:** `apps/web/src/routes/games/+page.svelte`, `apps/web/src/routes/games/[matchId]/+page.svelte`, `apps/web/src/lib/ui/GlobalNav.svelte`.
**Blocking:** no. The bug is in TS too.

## What happens

At 390px width, scrolled to the top:

- on `/games`, the nav covers the "Your saved tables" heading;
- on match detail, it covers most of the "← Back to your games" link (only "← Back to your" shows).

`GlobalNav.svelte` is `position: fixed; top: max(0.6rem, env(safe-area-inset-top))` and about 55px tall. The `.content` blocks on both pages start only 2rem (32px) from the top. The identity-gate card does not overlap.

The markup and CSS are the same as TS, so the port did not cause it.

## Why the W4 fix agent did not change it

W4 keeps the player-facing UI identical to TS unless the port forces a change. This is a design fix, not a port fix.

## Ask

In the UI fidelity wave, below 760px give these pages top padding of at least the nav height plus its offset, for example `calc(max(0.6rem, env(safe-area-inset-top)) + 4rem)`. Check `/games`, match detail and the other routes under the nav at 390px.
