<script lang="ts">
	import { page } from '$app/state';

	const links = [
		{ href: '/new', label: 'Play', title: 'Deal a new hand' },
		{ href: '/games', label: 'Your games', title: 'Your past games' },
		{ href: '/settings', label: 'Settings', title: 'Edit the opponents' }
	] as const;
	const here = $derived(page.url.pathname);
	const onLiveTable = $derived(page.route.id === '/play/[gameId]');
	const onHome = $derived(here === '/');
</script>

{#if !onLiveTable}
	<header class:home={onHome}>
		{#if onHome}<a class="brand" href="/" aria-label="Euchre home"
				>euchre<span aria-hidden="true">♣</span><small>THE CARD CLUB</small></a
			>{/if}
		<nav aria-label="Main navigation">
			{#if !onHome}<a href="/" aria-label="Euchre home" class="home-link">♣</a>{/if}
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
	header.home {
		position: absolute;
		top: 0;
		left: 0;
		right: 0;
		height: 104px;
		display: flex;
		align-items: center;
		justify-content: space-between;
		max-width: 1460px;
		margin: 0 auto;
		padding: 0 50px;
	}
	.brand {
		display: flex;
		align-items: center;
		text-decoration: none;
		color: #233e32;
		font: 600 40px/1 var(--font-serif);
		letter-spacing: -0.07em;
	}
	.brand > span {
		font-size: 25px;
		margin-left: 5px;
	}
	.brand small {
		font: 600 8px/1.4 var(--font-sans);
		letter-spacing: 0.18em;
		max-width: 48px;
		margin-left: 19px;
		border-left: 1px solid #a8b2a0;
		padding-left: 15px;
	}
	.home nav {
		background: transparent;
		border: 0;
		backdrop-filter: none;
		padding: 0;
		gap: 10px;
	}
	.home nav a {
		color: #42523e;
		border-radius: 6px;
	}
	.home nav a:hover {
		background: #e5e9d8;
		color: #153f36;
	}
	@media (max-width: 760px) {
		header.home {
			height: 86px;
			padding: 0 27px;
		}
		.brand {
			font-size: 32px;
		}
		.brand > span {
			font-size: 20px;
		}
		.brand small {
			display: none;
		}
		.home nav {
			gap: 0;
		}
		nav a {
			padding: 0 11px;
			font-size: 11px;
		}
	}
	@media (max-width: 380px) {
		header.home {
			padding: 0 22px;
		}
		.home nav a {
			padding: 0 8px;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		nav a {
			transition: none;
		}
	}
</style>
