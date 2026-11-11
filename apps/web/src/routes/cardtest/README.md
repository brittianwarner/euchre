# /cardtest — visual harness for the card layer

A dev-only route that renders `Card.svelte` in isolation: five face-up cards, a
face-down back, a `dimmed` card and a `highlighted` one.

It exists because a card can fail to appear for two completely different reasons —
the component is broken, or the scene is placing it out of frame — and those look
identical on the game table. Loading this route separates them in one step. It is
how the "faces don't render" report was traced to scene composition rather than to
`Card.svelte`, which turned out to be correct all along.

    bunx vite dev --port 5199   →   http://localhost:5199/cardtest
