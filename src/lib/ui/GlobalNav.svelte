<!--
  GlobalNav — the only way in or out of /games and /settings right now, so it
  has to work from every screen that has room for it: the landing page, the
  pre-game /play load, /games and /settings themselves.

  It deliberately does NOT render on the live table (`/play/[gameId]`): that
  page's own HUD already occupies all four corners plus top-centre by design
  (`+page.svelte`'s own doc comment maps ScoreBoard/status to top-left, the
  trick view to top-right, the rules "?" to top-centre, table talk to
  bottom-left and the action panel to bottom-right) — verified against
  screenshots at both 1440×900 and 390×844. A fifth floating corner would
  either collide with one of those or force re-verifying all of them, and
  this component owns none of that layout. The table's own "Deal a new game"
  link is the way out of a live hand; this nav picks up from there.

  Fixed, corner-docked, zero layout footprint elsewhere: it must never nudge
  the Threlte canvas or change its measured size, so this renders as a
  floating overlay, not a document-flow header. `pointer-events` is scoped to
  the pill itself so nothing under the transparent corner around it is ever
  blocked.
-->
<script lang="ts">
	import { page } from '$app/state';

	const links = [
		{ href: '/play', label: 'Table', title: 'Deal a new hand' },
		{ href: '/games', label: 'Games', title: 'Your past games' },
		{ href: '/settings', label: 'Settings', title: 'Edit the opponents' }
	] as const;

	const here = $derived(page.url.pathname);
	/** The live table owns every corner of its own HUD — see the doc comment above. */
	const onLiveTable = $derived(page.route.id === '/play/[gameId]');
</script>

{#if !onLiveTable}
	<nav class="global-nav" aria-label="Euchre">
		{#each links as link (link.href)}
			{@const active = here === link.href || (link.href !== '/play' && here.startsWith(link.href))}
			<a href={link.href} title={link.title} aria-current={active ? 'page' : undefined}>
				{link.label}
			</a>
		{/each}
	</nav>
{/if}

<style>
	.global-nav {
		position: fixed;
		top: max(0.6rem, env(safe-area-inset-top));
		right: max(0.6rem, env(safe-area-inset-right));
		z-index: 200;
		display: flex;
		gap: 0.3rem;
		padding: 0.3rem;
		border-radius: 999px;
		background: rgba(15, 20, 14, 0.78);
		border: 1px solid rgba(232, 194, 122, 0.3);
		box-shadow: 0 2px 12px rgba(0, 0, 0, 0.4);
		backdrop-filter: blur(6px);
		font-family: 'Source Serif 4', 'Iowan Old Style', Georgia, serif;
	}
	a {
		display: flex;
		align-items: center;
		min-height: 44px;
		min-width: 44px;
		justify-content: center;
		padding: 0 0.75rem;
		border-radius: 999px;
		color: #c9b89a;
		text-decoration: none;
		font-size: 0.82rem;
		font-weight: 600;
		letter-spacing: 0.01em;
		transition: background 160ms ease, color 160ms ease;
	}
	a:hover,
	a:focus-visible {
		background: rgba(232, 194, 122, 0.14);
		color: #f4ecd8;
	}
	a[aria-current='page'] {
		background: #3d6b38;
		color: #f4f7e8;
	}
	@media (prefers-reduced-motion: reduce) {
		a {
			transition: none;
		}
	}
	@media (max-width: 420px) {
		a {
			padding: 0 0.55rem;
			font-size: 0.76rem;
		}
	}
</style>
