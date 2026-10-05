<!--
  /settings — the AI persona editor. Three opponents, one shared house
  prompt, a handful of table preferences — all of it stored by
  `playerProfile.updateSettings`, none of it trusted by the model that reads
  it (see the disclaimer box below; it is not decoration, it is the actual
  security model).
-->
<script lang="ts">
	import {
		DIAL_MAX,
		DIAL_MIN,
		DIFFICULTIES,
		HOUSE_PROMPT_MAX_CHARS,
		PERSONA_BLURB_MAX_CHARS,
		PERSONA_NAME_MAX_CHARS,
		PERSONA_PROMPT_MAX_CHARS,
		type Difficulty
	} from '#lib/protocol/index.ts';
	import { AI_SEATS } from '#lib/actors/player-profile/types.ts';
	import IdentityGate from '#lib/ui/IdentityGate.svelte';
	import PlayingAs from '#lib/ui/PlayingAs.svelte';
	import { PERSONA_PRESETS, type PersonaPreset } from './presets';

	let { data, form } = $props();

	const SEAT_LABEL = { 1: 'West', 2: 'North · your partner', 3: 'East' } as const;

	const DIFFICULTY_EXPLAIN: Record<Difficulty, string> = {
		rookie: 'Plays a little unpredictably. Easygoing, forgiving of mistakes at the table.',
		casual: 'Solid, sensible play most of the time — the middle of the road.',
		expert: 'Weighs every legal option every time. The strongest seat at the table.'
	};

	interface PersonaForm {
		name: string;
		blurb: string;
		prompt: string;
		difficulty: Difficulty;
		aggression: number;
		risk: number;
		chattiness: number;
	}

	function seedForms(): PersonaForm[] {
		if (!data.settings) return [];
		return data.settings.personas.map((p) => ({
			name: p.name,
			blurb: p.blurb,
			prompt: p.prompt,
			difficulty: p.difficulty,
			aggression: p.aggression,
			risk: p.risk,
			chattiness: p.chattiness
		}));
	}

	function seedHousePrompt(): string {
		return data.settings?.housePrompt ?? '';
	}

	function seedTable() {
		return {
			pace: data.settings?.table.pace ?? 'normal',
			banter: data.settings?.table.banter ?? true,
			showRationale: data.settings?.table.showRationale ?? true,
			sound: data.settings?.table.sound ?? true,
			reduceMotion: data.settings?.table.reduceMotion ?? false
		};
	}

	let personaForms = $state(seedForms());
	let housePrompt = $state(seedHousePrompt());
	let table = $state(seedTable());

	function applyPreset(index: number, preset: PersonaPreset): void {
		const current = personaForms[index];
		personaForms[index] = {
			...current,
			blurb: preset.blurb,
			prompt: preset.prompt,
			difficulty: preset.difficulty,
			aggression: preset.aggression,
			risk: preset.risk,
			chattiness: preset.chattiness
		};
	}

	function pct(value: number): string {
		return `${Math.round(value * 100)}%`;
	}

	function fieldName(seat: number, field: string): string {
		return `seat${seat}_${field}`;
	}
</script>

<svelte:head>
	<title>Settings · Euchre</title>
</svelte:head>

<main class="page">
	<div class="content">
		{#if data.identity === null}
			<IdentityGate
				heading="Whose opponents are these?"
				lede="Enter your email address and the three seats you set up here will be waiting every time you play."
				errorMessage={form?.error ?? null}
			/>
		{:else}
			<header class="page-head">
				<p class="brand">Euchre</p>
				<h1>Settings</h1>
				<PlayingAs email={data.identity.email} />
			</header>

			{#if data.loadError}
				<p class="banner error">Couldn't open your settings just now. Try refreshing the page.</p>
			{:else if data.settings}
				<div class="disclaimer">
					<p>
						<strong>What this does and doesn't do:</strong> everything below shapes how your three
						opponents sound and how readily they bid or gamble.
						<strong
							>It cannot change the rules of euchre, let a seat see cards it shouldn't, or make an
							illegal move legal</strong
						> — those are enforced by the game itself and no amount of text here reaches them. Treat these
						prompts the way you'd treat a note passed to an actor, not a rulebook.
					</p>
				</div>

				<form method="POST" action="?/save" class="settings-form">
					{#if form?.error}
						<p class="banner error" role="alert">{form.error}</p>
					{/if}
					{#if form?.saved}
						<p class="banner success" role="status">Saved. Your next match will use these.</p>
					{/if}

					<section class="card">
						<h2>The table</h2>
						<p class="hint">How the game presents itself. Nothing here touches how anyone plays.</p>
						<div class="table-grid">
							<label class="field">
								<span>Pace</span>
								<select name="pace" bind:value={table.pace}>
									<option value="brisk">Brisk</option>
									<option value="normal">Normal</option>
									<option value="relaxed">Relaxed</option>
								</select>
							</label>
							<label class="check">
								<input type="checkbox" name="banter" bind:checked={table.banter} />
								<span>Let opponents talk at the table</span>
							</label>
							<label class="check">
								<input type="checkbox" name="showRationale" bind:checked={table.showRationale} />
								<span>Show each seat's reasoning after a hand</span>
							</label>
							<label class="check">
								<input type="checkbox" name="sound" bind:checked={table.sound} />
								<span>Sound</span>
							</label>
							<label class="check">
								<input type="checkbox" name="reduceMotion" bind:checked={table.reduceMotion} />
								<span>Reduce motion</span>
							</label>
						</div>
					</section>

					<section class="card">
						<h2>House prompt</h2>
						<p class="hint">
							Sets the overall mood of the table — applies quietly to all three opponents at once.
						</p>
						<textarea
							name="housePrompt"
							rows="3"
							maxlength={HOUSE_PROMPT_MAX_CHARS}
							bind:value={housePrompt}
						></textarea>
						<p class="count" class:near={housePrompt.length > HOUSE_PROMPT_MAX_CHARS * 0.9}>
							{housePrompt.length} / {HOUSE_PROMPT_MAX_CHARS}
						</p>
					</section>

					{#each AI_SEATS as seat, index (seat)}
						{@const pf = personaForms[index]}
						<section class="card persona">
							<h2>{SEAT_LABEL[seat]}</h2>

							<label class="field">
								<span>Name</span>
								<input
									type="text"
									name={fieldName(seat, 'name')}
									maxlength={PERSONA_NAME_MAX_CHARS}
									bind:value={pf.name}
								/>
							</label>
							<p class="count" class:near={pf.name.length > PERSONA_NAME_MAX_CHARS * 0.9}>
								{pf.name.length} / {PERSONA_NAME_MAX_CHARS}
							</p>

							<label class="field">
								<span>One-line blurb (shown at the table)</span>
								<input
									type="text"
									name={fieldName(seat, 'blurb')}
									maxlength={PERSONA_BLURB_MAX_CHARS}
									bind:value={pf.blurb}
								/>
							</label>
							<p class="count" class:near={pf.blurb.length > PERSONA_BLURB_MAX_CHARS * 0.9}>
								{pf.blurb.length} / {PERSONA_BLURB_MAX_CHARS}
							</p>

							<fieldset class="dials">
								<label class="dial">
									<span>How eagerly they bid — {pct(pf.aggression)}</span>
									<input
										type="range"
										name={fieldName(seat, 'aggression')}
										min={DIAL_MIN}
										max={DIAL_MAX}
										step="0.05"
										bind:value={pf.aggression}
									/>
								</label>
								<label class="dial">
									<span>How much they'll gamble on a call — {pct(pf.risk)}</span>
									<input
										type="range"
										name={fieldName(seat, 'risk')}
										min={DIAL_MIN}
										max={DIAL_MAX}
										step="0.05"
										bind:value={pf.risk}
									/>
								</label>
								<label class="dial">
									<span>How much they talk — {pct(pf.chattiness)}</span>
									<input
										type="range"
										name={fieldName(seat, 'chattiness')}
										min={DIAL_MIN}
										max={DIAL_MAX}
										step="0.05"
										bind:value={pf.chattiness}
									/>
								</label>
							</fieldset>

							<label class="field">
								<span>Difficulty</span>
								<select name={fieldName(seat, 'difficulty')} bind:value={pf.difficulty}>
									{#each DIFFICULTIES as level (level)}
										<option value={level}>{level}</option>
									{/each}
								</select>
							</label>
							<p class="hint small">{DIFFICULTY_EXPLAIN[pf.difficulty]}</p>

							<div class="presets">
								<p class="hint small">
									One-tap starting points — fills in the blurb, prompt and dials below, doesn't save
									until you do:
								</p>
								<div class="chips">
									{#each PERSONA_PRESETS as preset (preset.id)}
										<button
											type="button"
											class="chip"
											title={preset.hint}
											onclick={() => applyPreset(index, preset)}
										>
											{preset.label}
										</button>
									{/each}
								</div>
							</div>

							<label class="field">
								<span>Character prompt — untrusted text, style only</span>
								<textarea
									name={fieldName(seat, 'prompt')}
									rows="4"
									maxlength={PERSONA_PROMPT_MAX_CHARS}
									bind:value={pf.prompt}
								></textarea>
							</label>
							<p class="count" class:near={pf.prompt.length > PERSONA_PROMPT_MAX_CHARS * 0.9}>
								{pf.prompt.length} / {PERSONA_PROMPT_MAX_CHARS}
							</p>
						</section>
					{/each}

					<button type="submit" class="save">Save changes</button>
				</form>
			{/if}
		{/if}
	</div>
</main>

<style>
	.page {
		width: 100%;
		min-height: 100dvh;
		color: #f2e8d5;
		font-family: 'Source Serif 4', 'Iowan Old Style', Georgia, serif;
		background: #0b0906;
	}
	.content {
		max-width: 42rem;
		margin: 0 auto;
		padding: 2rem 1.25rem 5rem;
	}
	.page-head {
		margin-bottom: 1.25rem;
	}
	.brand {
		margin: 0;
		font-size: 1rem;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: #93876f;
	}
	h1 {
		margin: 0.15rem 0 0.9rem;
		font-size: clamp(1.8rem, 5vw, 2.4rem);
	}
	.banner {
		margin: 0 0 1.25rem;
		padding: 0.85rem 1rem;
		border-radius: 0.6rem;
		background: rgba(15, 20, 14, 0.6);
		border: 1px solid rgba(232, 194, 122, 0.22);
		color: #c9b89a;
	}
	.banner.error {
		background: rgba(74, 32, 24, 0.5);
		border-color: rgba(224, 140, 110, 0.4);
		color: #f7e2d8;
	}
	.banner.success {
		background: rgba(61, 107, 56, 0.35);
		border-color: rgba(183, 220, 170, 0.4);
		color: #dff0d6;
	}
	.disclaimer {
		margin: 0 0 1.5rem;
		padding: 0.9rem 1rem;
		border-radius: 0.6rem;
		background: rgba(232, 194, 122, 0.08);
		border: 1px solid rgba(232, 194, 122, 0.3);
	}
	.disclaimer p {
		margin: 0;
		font-family: system-ui, sans-serif;
		font-size: 0.85rem;
		line-height: 1.55;
		color: #e8dcc6;
	}
	.disclaimer strong {
		color: #f2e8d5;
	}
	.settings-form {
		display: grid;
		gap: 1.25rem;
	}
	.card {
		padding: 1.1rem 1.15rem;
		border-radius: 0.75rem;
		background: rgba(15, 20, 14, 0.55);
		border: 1px solid rgba(232, 194, 122, 0.18);
	}
	.card h2 {
		margin: 0 0 0.3rem;
		font-size: 1.15rem;
	}
	.hint {
		margin: 0 0 0.9rem;
		font-family: system-ui, sans-serif;
		color: #93876f;
		font-size: 0.85rem;
		line-height: 1.5;
	}
	.hint.small {
		margin: 0.25rem 0 0.6rem;
		font-size: 0.78rem;
	}
	.table-grid {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 0.6rem 1rem;
	}
	.field {
		display: block;
		margin-bottom: 0.15rem;
	}
	.field span {
		display: block;
		margin-bottom: 0.3rem;
		font-family: system-ui, sans-serif;
		font-size: 0.82rem;
		color: #c9b89a;
	}
	input[type='text'],
	select,
	textarea {
		width: 100%;
		min-height: 44px;
		padding: 0.5rem 0.7rem;
		border-radius: 0.5rem;
		border: 1px solid #6a5638;
		background: #241b10;
		color: #f2e8d5;
		font: inherit;
		font-size: 0.95rem;
		box-sizing: border-box;
	}
	textarea {
		resize: vertical;
		font-family: system-ui, sans-serif;
		line-height: 1.5;
	}
	input:focus-visible,
	select:focus-visible,
	textarea:focus-visible {
		outline: 2px solid #e8c27a;
		outline-offset: 1px;
	}
	.count {
		margin: 0.2rem 0 0.9rem;
		font-family: system-ui, sans-serif;
		font-size: 0.75rem;
		color: #6d6250;
		text-align: right;
	}
	.count.near {
		color: #e0a06e;
	}
	.check {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		min-height: 44px;
		font-family: system-ui, sans-serif;
		font-size: 0.88rem;
		color: #e8dcc6;
	}
	.check input {
		width: 20px;
		height: 20px;
		accent-color: #3d6b38;
	}
	.dials {
		border: none;
		margin: 0.4rem 0 0.9rem;
		padding: 0;
		display: grid;
		gap: 0.75rem;
	}
	.dial span {
		display: block;
		margin-bottom: 0.3rem;
		font-family: system-ui, sans-serif;
		font-size: 0.82rem;
		color: #c9b89a;
	}
	.dial input[type='range'] {
		width: 100%;
		accent-color: #e8c27a;
		min-height: 44px;
	}
	.presets {
		margin: 0.3rem 0 0.9rem;
	}
	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
	}
	.chip {
		min-height: 44px;
		padding: 0 0.7rem;
		border-radius: 999px;
		border: 1px solid #6a5638;
		background: #241b10;
		color: #d9cbb0;
		font-family: system-ui, sans-serif;
		font-size: 0.78rem;
		cursor: pointer;
	}
	.chip:hover {
		border-color: #e8c27a;
		color: #f2e8d5;
	}
	.save {
		justify-self: start;
		min-height: 44px;
		padding: 0 1.4rem;
		border-radius: 0.5rem;
		border: 1px solid #4a7f44;
		background: #3d6b38;
		color: #f4f7e8;
		font: inherit;
		font-weight: 600;
		font-size: 1rem;
		cursor: pointer;
	}
	.save:hover {
		background: #4a7f44;
	}
	@media (max-width: 480px) {
		.table-grid {
			grid-template-columns: 1fr;
		}
	}
</style>
