# `$lib/ui`

DOM-only play surface for M2 (before Threlte).

| Component | Role |
|---|---|
| `ScoreBoard.svelte` | Score, trump, turn |
| `TrickView.svelte` | Cards in the current trick |
| `BidPanel.svelte` | Cut / order-up / call |
| `DiscardPanel.svelte` | Dealer bury |
| `HandA11y.svelte` | Focusable hand buttons |

All interactions call `onPlay(moveId)` with ids from `view.legal`.
