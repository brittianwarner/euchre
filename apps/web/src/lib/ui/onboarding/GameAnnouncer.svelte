<!--
  GameAnnouncer — the accessible narration of the table.

  A visually-hidden `aria-live="polite"` region that turns every `Step` the
  server sends into one plain-English sentence (docs/04-FRONTEND-UX.md §13).
  This is the permanent screen-reader record of "what just happened", separate
  from — and complementary to — the visible talk log: the table's engine-
  authored `call` lines (`sayForBid()`'s "Order it up.", "I assist.", the
  "Euchre!"/"March!" calls) are already broadcast as `chat` and already sit in
  an `aria-live="polite"` region in `+page.svelte`'s corner-bl, so this
  component deliberately says nothing for a `bid` step — repeating it here
  would double-announce the exact same words. Every OTHER step type has no
  spoken counterpart anywhere in the app until this component exists, so this
  is where "announce every game event" actually gets satisfied.

  Pure `$derived` the whole way down: no `$effect`. Reassigning the live
  region's text is a plain reactive write, and the browser's own aria-live
  machinery is what notices the DOM mutation and speaks it — nothing here
  needs to reach into the DOM by hand.
-->
<script lang="ts">
	import {
		DEFAULT_ENGINE_CONFIG,
		SUIT_NAME,
		SUIT_NOUN,
		bowersOf,
		cardNameLower,
		isGameOver,
		isInTheBarn,
		teamOf
	} from '#lib/euchre/index.ts';
	import type { HandResult, PublicGameView, Seat, Step } from '#lib/protocol/index.ts';

	interface Props {
		/** Every step since the last sync this connection saw — see `SyncEvent.steps`. */
		steps: readonly Step[];
		view: PublicGameView | null;
	}

	let { steps, view }: Props = $props();

	/** Matches the compass names already spoken in `+page.svelte`'s talk log. */
	const SEAT_NAME = [
		'You',
		'Left opponent',
		'Your partner',
		'Right opponent'
	] as const satisfies readonly string[];

	const RESULT_LABEL: Readonly<Record<HandResult, string>> = {
		point: 'Point.',
		lone_point: 'Lone point.',
		march: 'March!',
		lone_march: 'Lone march!',
		euchre: 'Euchre!',
		throw_in: 'Thrown in.'
	};

	function who(seat: Seat, you: Seat): string {
		return seat === you ? 'You' : (SEAT_NAME[seat] ?? 'Someone');
	}

	/** Picks the third-person form unless `seat` is the listener themself. */
	function verb(seat: Seat, you: Seat, firstPerson: string, thirdPerson: string): string {
		return seat === you ? firstPerson : thirdPerson;
	}

	function sentenceFor(step: Step, view: PublicGameView): string | null {
		const you = view.you;
		switch (step.t) {
			case 'cut': {
				const say = step.cut ? 'Bump.' : "Run 'em.";
				return `${who(step.seat, you)}: ${say}`;
			}

			case 'dealt':
				// One `dealt` step arrives per deal packet (several per hand); only the
				// first is worth a sentence; the rest are silently skipped by the caller.
				return view.hand.length > 0
					? `Cards dealt. Your hand: ${view.hand.map(cardNameLower).join(', ')}.`
					: null;

			case 'upCardTurned':
				return `Up-card is the ${cardNameLower(step.card)}.`;

			case 'turnedDown':
				return `${SUIT_NAME[step.suit]} turned down. ${SUIT_NAME[step.suit]} may not be named.`;

			case 'trumpSet': {
				const [, left] = bowersOf(step.suit);
				const madeBy =
					step.makerSeat === you
						? `${SUIT_NAME[step.suit]} are trump. You called it.`
						: `${SUIT_NAME[step.suit]} are trump, made by ${SEAT_NAME[step.makerSeat]}.`;
				const alone =
					step.aloneSeat === null
						? ''
						: step.aloneSeat === you
							? ' You are going alone.'
							: ` ${SEAT_NAME[step.aloneSeat]} is going alone.`;
				return `${madeBy}${alone} The ${cardNameLower(left)} is now a ${SUIT_NOUN[step.suit]}.`;
			}

			case 'dealerDiscarded':
				return step.seat === you
					? 'You discard face down.'
					: `${SEAT_NAME[step.seat]} discards face down.`;

			case 'cardPlayed':
				return `${who(step.seat, you)} ${verb(step.seat, you, 'play', 'plays')} the ${cardNameLower(step.card)}.`;

			case 'trickWon': {
				const trickNo = step.index + 1;
				const lead =
					trickNo >= 5
						? ''
						: step.winnerSeat === you
							? ' Your lead.'
							: teamOf(step.winnerSeat) === teamOf(you)
								? ' Partner leads.'
								: ` ${SEAT_NAME[step.winnerSeat]} leads.`;
				return `${who(step.winnerSeat, you)} ${verb(step.winnerSeat, you, 'take', 'takes')} the trick. ${trickNo} of five.${lead}`;
			}

			case 'handScored': {
				const gameTo = DEFAULT_ENGINE_CONFIG.gameTo;
				const scoreLine =
					step.delta[0] > 0
						? `You score ${step.delta[0]}.`
						: step.delta[1] > 0
							? `They score ${step.delta[1]}.`
							: 'Nobody scores.';
				const tally = ` ${step.score[0]} to ${step.score[1]}.`;
				const barn = isGameOver(step.score, gameTo)
					? ''
					: isInTheBarn(step.score, 0, gameTo)
						? ' You are in the barn.'
						: isInTheBarn(step.score, 1, gameTo)
							? ' They are in the barn.'
							: '';
				return `${RESULT_LABEL[step.result]} ${scoreLine}${tally}${barn}`;
			}

			case 'throwIn':
				return 'All passed twice. Thrown in — the deal moves left.';

			case 'gameWon':
				return `${step.team === teamOf(you) ? 'You win' : 'They win'}, ${step.score[0]} to ${step.score[1]}.`;

			case 'bid':
				// Already spoken verbatim via the `chat` ('call') channel — see this
				// component's own doc comment.
				return null;

			default:
				return null;
		}
	}

	const announcement = $derived.by((): string => {
		if (view === null || steps.length === 0) return '';
		const seen = new Set<Step['t']>();
		const lines: string[] = [];
		for (const step of steps) {
			if (step.t === 'dealt' && seen.has('dealt')) continue; // one "cards dealt" line per hand, not per packet
			seen.add(step.t);
			const line = sentenceFor(step, view);
			if (line !== null) lines.push(line);
		}
		return lines.join(' ');
	});
</script>

<div class="sr-only" role="status" aria-live="polite" aria-atomic="true">
	{announcement}
</div>

<style>
	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip: rect(0, 0, 0, 0);
		white-space: nowrap;
		border: 0;
	}
</style>
