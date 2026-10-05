<svelte:head>
	<title>Euchre — A little nerve. A great hand.</title>
	<meta
		name="description"
		content="Your new favorite card table. Play classic partnership euchre with computer opponents, a beautiful 3D table, and a little friendly competition."
	/>
</svelte:head>

<main>
	<section class="hero" aria-labelledby="hero-title">
		<div class="hero-grain" aria-hidden="true"></div>
		<div class="hero-copy">
			<p class="eyebrow"><span></span> THE TABLE IS YOURS</p>
			<h1 id="hero-title">A little nerve.<br />A great <em>hand.</em></h1>
			<p class="lede">
				Five cards. Four seats. Endless good calls.<br class="desktop-break" /> The classic card game,
				with a fresh seat at the table.
			</p>
			<div class="hero-actions">
				<a class="deal-button" href="/new">Let’s play <span aria-hidden="true">↗</span></a>
				<a class="rules-link" href="#how-to-play"
					>New to euchre? <span aria-hidden="true">↓</span></a
				>
			</div>
			<p class="play-note">Play against computer opponents · No account needed</p>
		</div>
		<div class="card-scene">
			<img
				class="hero-art"
				src="/art/euchre-hero.webp"
				width="1200"
				height="1000"
				alt="Classic playing cards and a dealer button on a green felt table, with both red jacks at the front"
				fetchpriority="high"
				decoding="async"
			/>
			<div class="scene-caption">GOOD COMPANY. BETTER CARDS.</div>
			<div class="bower-stamp" aria-hidden="true">
				<span>THE RIGHT</span><strong>J<span>♥</span></strong><span>KIND OF LUCK</span>
			</div>
		</div>
		<div class="hero-bottom">
			<span>A CLASSIC, DEALT DIFFERENTLY.</span><span aria-hidden="true">♠ <i>♥</i> ♣ <i>♦</i></span
			><span>FIRST TO TEN. MAKE IT COUNT.</span>
		</div>
	</section>

	<section class="introduction" aria-labelledby="intro-title">
		<div>
			<p class="eyebrow">SMALL DECK. BIG PERSONALITY.</p>
			<h2 id="intro-title">All the feeling.<br />None of the shuffling.</h2>
		</div>
		<div class="intro-copy">
			<p>
				A proper game of partnership euchre, made for the browser. Read the table, trust your
				partner, and know when to go it alone.
			</p>
			<p class="muted">
				Settle into a 3D table with three computer players. Your next good hand is one deal away.
			</p>
		</div>
	</section>

	<section class="how-to" id="how-to-play" aria-labelledby="rules-title">
		<div class="section-heading">
			<p class="eyebrow">PULL UP A CHAIR</p>
			<h2 id="rules-title">Easy to learn.<br /><em>Hard to put down.</em></h2>
		</div>
		<div class="rule-cards">
			<article>
				<span class="rule-number">01 /</span><span class="rule-suit" aria-hidden="true">♣</span>
				<h3>Find your partner.</h3>
				<p>
					Four players. Two teams. The player across the table is on your side. You each get five
					cards.
				</p>
			</article>
			<article>
				<span class="rule-number">02 /</span><span class="rule-suit red-suit" aria-hidden="true"
					>♦</span
				>
				<h3>Make the call.</h3>
				<p>
					Choose a trump suit or pass. The team that calls trump needs to take at least three of the
					five tricks.
				</p>
			</article>
			<article>
				<span class="rule-number">03 /</span><span class="rule-suit" aria-hidden="true">♠</span>
				<h3>Play your hand.</h3>
				<p>
					Follow suit if you can. The highest card wins the trick, and trump beats everything else.
					First team to ten wins.
				</p>
			</article>
		</div>
		<details>
			<summary>The jacks have a little secret <span aria-hidden="true">+</span></summary>
			<div class="rule-detail">
				<p>
					The jack of trump is the <strong>right bower</strong>, the strongest card. The other jack
					of the same color is the <strong>left bower</strong>, the second strongest. Both belong to
					the trump suit.
				</p>
				<p>
					Take three or four tricks after calling trump for one point, or all five for two. Get
					stopped and the other team earns two. Feeling bold? Go alone and take all five for four
					points.
				</p>
			</div>
		</details>
	</section>
	<footer>
		<a class="footer-brand" href="/">euchre<span>♣</span></a>
		<p>A good hand is better shared.</p>
		<a href="/new">See you at the table <span aria-hidden="true">↗</span></a>
	</footer>
</main>

<style>
	main {
		background: var(--club-cream);
		color: var(--club-ink);
		padding: 104px 28px 0;
	}
	.hero {
		position: relative;
		overflow: hidden;
		min-height: 620px;
		border-radius: 18px;
		background: #153f36;
		color: #f5f1e5;
		display: grid;
		grid-template-columns: 1.1fr 1fr;
		align-items: center;
		padding: 70px 5.3% 96px;
		isolation: isolate;
	}
	.hero-grain {
		position: absolute;
		inset: 0;
		opacity: 0.25;
		z-index: -1;
		background-image: radial-gradient(#bdd4b1 0.5px, transparent 0.5px);
		background-size: 5px 5px;
		mask-image: linear-gradient(120deg, transparent, #000);
	}
	.hero-copy {
		position: relative;
		z-index: 2;
	}
	.eyebrow {
		font: 600 10px/1.5 var(--font-sans);
		letter-spacing: 0.18em;
		margin: 0 0 25px;
	}
	.hero .eyebrow {
		color: #d5e7bb;
		display: flex;
		align-items: center;
		gap: 9px;
	}
	.eyebrow > span {
		height: 6px;
		width: 6px;
		border-radius: 50%;
		background: var(--club-lime);
		box-shadow: 0 0 0 4px #d4ed9b15;
	}
	h1 {
		font: 500 clamp(52px, 6.2vw, 94px)/1.02 var(--font-serif);
		letter-spacing: -0.055em;
		margin: 0;
	}
	h1 em {
		color: var(--club-lime);
		font-weight: 400;
	}
	.lede {
		margin: 25px 0 29px;
		color: #d2dcd4;
		font: 400 15px/1.75 var(--font-sans);
	}
	.hero-actions {
		display: flex;
		align-items: center;
		gap: 25px;
	}
	.deal-button {
		display: inline-flex;
		align-items: center;
		gap: 48px;
		border-radius: 7px;
		background: var(--club-lime);
		padding: 17px 23px;
		text-decoration: none;
		color: #143e32;
		font-weight: 650;
		font-size: 14px;
		transition:
			background 0.2s,
			transform 0.2s;
	}
	.deal-button:hover {
		background: #e3f6be;
		transform: translateY(-2px);
	}
	.deal-button span {
		font-size: 21px;
		line-height: 1;
	}
	.rules-link {
		color: #f4f1e5;
		font-size: 12px;
		text-underline-offset: 5px;
	}
	.rules-link span {
		margin-left: 8px;
	}
	.play-note {
		font-size: 10px;
		color: #a8bcb0;
		margin: 18px 0 0;
	}
	.card-scene {
		position: relative;
		height: 395px;
		min-width: 0;
	}
	.hero-art {
		position: absolute;
		top: 50%;
		left: 50%;
		width: 135%;
		max-width: none;
		height: auto;
		transform: translate(-50%, -50%);
		mask-image: radial-gradient(ellipse 64% 59% at 50% 50%, black 48%, transparent 95%);
	}
	.scene-caption {
		position: absolute;
		top: 0;
		left: 50%;
		transform: translateX(-50%);
		font-size: 8px;
		letter-spacing: 0.22em;
		color: #a1b9a6;
		white-space: nowrap;
	}
	.bower-stamp {
		position: absolute;
		right: 0;
		bottom: -4px;
		width: 100px;
		height: 100px;
		border-radius: 50%;
		background: var(--club-lime);
		color: #153f36;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		transform: rotate(14deg);
		box-shadow: 0 0 0 6px #153f36;
	}
	.bower-stamp > span {
		font-size: 7px;
		letter-spacing: 0.11em;
		font-weight: 700;
	}
	.bower-stamp strong {
		font: 36px/1.2 var(--font-serif);
	}
	.bower-stamp strong span {
		font-size: 25px;
	}
	.hero-bottom {
		position: absolute;
		bottom: 26px;
		left: 5.3%;
		right: 5.3%;
		border-top: 1px solid #c6d7b92b;
		padding-top: 20px;
		display: flex;
		justify-content: space-between;
		align-items: center;
		color: #9db3a4;
		font-size: 8px;
		letter-spacing: 0.16em;
	}
	.hero-bottom > span:nth-child(2) {
		font-size: 18px;
		letter-spacing: 7px;
		color: #d5e7bb;
		line-height: 1;
	}
	.hero-bottom i {
		font-style: normal;
		color: #a8b893;
	}
	.introduction {
		max-width: 1120px;
		margin: 0 auto;
		padding: 85px 24px;
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 100px;
		align-items: center;
	}
	.introduction .eyebrow {
		color: #6d7865;
		margin-bottom: 15px;
	}
	h2 {
		font: 500 clamp(34px, 3.4vw, 48px)/1.12 var(--font-serif);
		letter-spacing: -0.045em;
		margin: 0;
	}
	.intro-copy p {
		font-size: 15px;
		line-height: 1.8;
		margin: 0;
	}
	.intro-copy .muted {
		color: #72766a;
		font-size: 13px;
		margin-top: 12px;
	}
	.how-to {
		border-top: 1px solid #d9ddcf;
		padding: 66px 24px 72px;
		max-width: 1120px;
		margin: 0 auto;
		scroll-margin-top: 35px;
	}
	.section-heading {
		display: flex;
		align-items: start;
		justify-content: space-between;
		margin-bottom: 35px;
	}
	.section-heading .eyebrow {
		color: #6d7865;
		padding-top: 9px;
	}
	.section-heading h2 {
		width: 50%;
	}
	h2 em {
		font-weight: 400;
	}
	.rule-cards {
		display: grid;
		grid-template-columns: repeat(3, 1fr);
		gap: 16px;
	}
	article {
		position: relative;
		background: #e9ecdf;
		border-radius: 10px;
		padding: 28px;
	}
	.rule-number {
		font-size: 10px;
		color: #6b7664;
		letter-spacing: 0.08em;
	}
	.rule-suit {
		position: absolute;
		right: 25px;
		top: 16px;
		font-size: 36px;
	}
	.red-suit {
		color: #b44835;
	}
	h3 {
		font: 500 26px/1.2 var(--font-serif);
		letter-spacing: -0.025em;
		margin: 37px 0 12px;
	}
	article p {
		font-size: 12px;
		line-height: 1.8;
		color: #5f695d;
		margin: 0;
	}
	details {
		border-top: 1px solid #d9ddcf;
		border-bottom: 1px solid #d9ddcf;
		margin-top: 26px;
	}
	summary {
		display: flex;
		justify-content: space-between;
		align-items: center;
		cursor: pointer;
		padding: 22px 0;
		font-size: 13px;
		font-weight: 550;
		list-style: none;
	}
	summary::-webkit-details-marker {
		display: none;
	}
	summary span {
		font-size: 23px;
		font-weight: 400;
	}
	details[open] summary span {
		transform: rotate(45deg);
	}
	.rule-detail {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 28px;
		padding-bottom: 24px;
		font-size: 13px;
		line-height: 1.8;
		color: #5f695d;
	}
	.rule-detail p {
		margin: 0;
	}
	footer {
		max-width: 1120px;
		margin: auto;
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 20px;
		padding: 0 24px 34px;
	}
	.footer-brand {
		font: 600 32px/1 var(--font-serif);
		letter-spacing: -0.065em;
		text-decoration: none;
	}
	.footer-brand span {
		font-size: 20px;
		margin-left: 5px;
	}
	footer p {
		font-size: 11px;
		color: #737b6b;
	}
	footer > a:last-child {
		font-size: 11px;
		text-underline-offset: 5px;
	}
	@media (min-width: 1500px) {
		main {
			padding-right: max(28px, calc((100vw - 1460px) / 2));
			padding-left: max(28px, calc((100vw - 1460px) / 2));
		}
	}
	@media (max-width: 1050px) {
		.hero {
			padding-left: 5%;
			padding-right: 5%;
		}
		.hero-actions {
			gap: 16px;
		}
		.deal-button {
			gap: 30px;
		}
		.introduction {
			gap: 40px;
		}
	}
	@media (max-width: 760px) {
		main {
			padding: 86px 14px 0;
		}
		.hero {
			grid-template-columns: 1fr;
			padding: 43px 28px 76px;
			gap: 0;
		}
		h1 {
			font-size: clamp(52px, 10vw, 72px);
		}
		.lede {
			font-size: 14px;
			margin-top: 20px;
		}
		.card-scene {
			height: 310px;
			margin-top: 14px;
		}
		.scene-caption {
			display: none;
		}
		.bower-stamp {
			right: -10px;
			bottom: 0;
		}
		.hero-bottom {
			left: 28px;
			right: 28px;
			bottom: 22px;
		}
		.hero-bottom > span:last-child {
			display: none;
		}
		.introduction {
			padding: 52px 14px;
			gap: 24px;
			grid-template-columns: 1fr;
		}
		.intro-copy {
			max-width: 470px;
		}
		.how-to {
			padding: 43px 14px;
		}
		.section-heading {
			display: block;
		}
		.section-heading h2 {
			width: auto;
		}
		.rule-cards {
			grid-template-columns: 1fr;
			gap: 12px;
		}
		article {
			padding: 24px;
		}
		h3 {
			margin-top: 19px;
		}
		.rule-detail {
			grid-template-columns: 1fr;
			gap: 14px;
		}
		footer {
			padding: 8px 14px 28px;
			flex-wrap: wrap;
		}
		footer p {
			display: none;
		}
	}
	@media (max-width: 380px) {
		.hero {
			padding-left: 22px;
			padding-right: 22px;
		}
		.hero-actions {
			flex-wrap: wrap;
		}
		.hero-bottom > span:first-child {
			font-size: 7px;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.deal-button {
			transition: none;
		}
	}
</style>
