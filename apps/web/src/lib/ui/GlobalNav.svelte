<script lang="ts">
	import { page } from '$app/state';

	const links = [
		{ href: '/new', label: 'Play', title: 'Deal a new hand' },
		{ href: '/games', label: 'Your games', title: 'Your past games' },
		{ href: '/settings', label: 'Settings', title: 'Edit the opponents' }
	] as const;
	const here = $derived(page.url.pathname);
	// No nav on the home page (Play now and Download only) or on the live table,
	// whose header has its own links.
	const hidden = $derived(here === '/' || page.route.id === '/play/[gameId]');
</script>

{#if !hidden}
	<header>
		<nav aria-label="Main navigation">
			<a href="/" aria-label="Euchre home" class="home-link">♣</a>
			{#each links as link (link.href)}
				{@const active = here === link.href || (link.href !== '/new' && here.startsWith(link.href))}
				<a href={link.href} title={link.title} aria-current={active ? 'page' : undefined}
					>{link.label}</a
				>
			{/each}
		</nav>
	</header>
{/if}

<style>
	header {
		position: fixed;
		top: max(0.6rem, env(safe-area-inset-top));
		right: max(0.6rem, env(safe-area-inset-right));
		z-index: 200;
		font-family: var(--font-sans);
	}
	nav {
		display: flex;
		align-items: center;
		gap: 3px;
		padding: 5px;
		border-radius: 999px;
		background: #15352eee;
		border: 1px solid #aec29338;
		backdrop-filter: blur(10px);
	}
	nav a {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-height: 44px;
		min-width: 44px;
		padding: 0 17px;
		border-radius: 999px;
		color: #d9e3d4;
		font-size: 12px;
		font-weight: 550;
		text-decoration: none;
		transition:
			background 160ms,
			color 160ms;
	}
	nav a:hover {
		background: #d4ed9b1a;
		color: #f5f5e9;
	}
	nav a[aria-current='page'] {
		background: #d4ed9b;
		color: #153f36;
	}
	.home-link {
		font-size: 20px;
	}
	@media (max-width: 760px) {
		nav a {
			padding: 0 11px;
			font-size: 11px;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		nav a {
			transition: none;
		}
	}
</style>
