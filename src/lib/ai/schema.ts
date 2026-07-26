/**
 * The output schema — the mechanism that makes an illegal AI move unrepresentable.
 *
 * This is the single most important file in `$lib/ai`, and it is deliberately the
 * smallest. Everything else here is quality; this is integrity.
 *
 * `moveId` is `z.enum(ids)` where `ids` is built **fresh for every decision** from
 * the candidate set the table computed with `legalMoves()`. Not a `z.string()`
 * validated afterwards, not a regex, not free text parsed by us. The model's
 * structured-output decoding is constrained to that enum, so:
 *
 * - a persona that says "always play the jack of spades" cannot produce
 *   `play:JS` when it is not in the set — the value is not in the schema;
 * - a model that hallucinates a plausible-looking id fails generation rather than
 *   emitting it, and the ladder falls to the heuristic with zero state effect;
 * - a prompt-injection payload has nothing to inject *into*: the legal set is not
 *   assembled from anything the model or the persona can influence.
 *
 * Two rules keep that true, and both are cheap to check in review:
 *
 * 1. `ids` must come from `LegalMove[]` produced by the engine for this seat at
 *    this instant. Never from the model, never from a payload, never cached across
 *    decisions.
 * 2. `moveId` stays an enum. If a future edit widens it to `z.string()` "to handle
 *    a provider that struggles with enums", the structural guarantee is gone and
 *    only the table's independent re-derivation remains.
 *
 * No tools are used, and none should be: a tool call is another surface with
 * another parser, and it buys nothing here.
 */

import { z } from 'zod';
import type { AIDecisionKind, LegalMove, LegalMoveId } from '$lib/protocol';

/** `why` caps, per path. Bids get a little more room because they are journalled. */
const WHY_MAX_BID = 110;
const WHY_MAX_PLAY = 90;

/**
 * Thrown when a caller asks for a schema over an empty candidate set.
 *
 * `z.enum([])` is not a usable schema, and an empty legal set means the caller has
 * already violated engine invariant V19 ("`legal` is never empty when it is your
 * turn"). Failing loudly here beats emitting a schema that can never validate.
 */
export class EmptyCandidateSetError extends Error {
	constructor() {
		super('cannot build a move schema over an empty candidate set');
		this.name = 'EmptyCandidateSetError';
	}
}

/**
 * The enum of permitted move ids for exactly one decision.
 *
 * Exported on its own so a test can assert that the enum's members and the
 * rendered `LEGAL:` block came from the same array.
 */
export function moveIdEnum(ids: readonly LegalMoveId[]) {
	if (ids.length === 0) throw new EmptyCandidateSetError();
	return z.enum(ids);
}

/** Pull the ids out of a candidate set. The only supported way to feed `moveIdEnum`. */
export function idsOf(candidates: readonly LegalMove[]): readonly LegalMoveId[] {
	return candidates.map((m) => m.id);
}

/**
 * The shape every decision returns.
 *
 * `confidence` is optional on all three paths rather than required on bids. The
 * spec sketches three schemas differing in that field; collapsing them to one TS
 * type is worth more than the distinction, because `confidence` is advisory,
 * never drives control flow, and a model that omits it should not fail generation
 * and cost a fallback. The `why` cap still varies by path, which is the part that
 * affects tokens.
 */
export interface DecisionObject {
	readonly moveId: LegalMoveId;
	readonly why: string;
	readonly confidence?: number;
}

function schemaFor(ids: readonly LegalMoveId[], whyMax: number) {
	return z.object({
		moveId: moveIdEnum(ids),
		why: z.string().max(whyMax),
		confidence: z.number().min(0).max(1).optional()
	});
}

/** Round-one and round-two bidding, including the loner ids. */
export function bidSchema(ids: readonly LegalMoveId[]) {
	return schemaFor(ids, WHY_MAX_BID);
}

/** The dealer's face-down discard after the up-card is ordered up. */
export function discardSchema(ids: readonly LegalMoveId[]) {
	return schemaFor(ids, WHY_MAX_BID);
}

/** One card into the current trick. */
export function playSchema(ids: readonly LegalMoveId[]) {
	return schemaFor(ids, WHY_MAX_PLAY);
}

/**
 * Dispatch by decision kind.
 *
 * Note there is no separate "go alone" schema. The `LegalMoveId` vocabulary
 * already encodes the loner (`orderUp+alone`, `call:H+alone`) and the engine emits
 * those ids only when the loner is actually legal for that seat at that moment. A
 * second call would cost a round trip inside the bid budget and — worse — would
 * let a model declare `alone` without a successful call.
 */
export function decisionSchema(kind: AIDecisionKind, ids: readonly LegalMoveId[]) {
	switch (kind) {
		case 'play':
			return playSchema(ids);
		case 'discard':
			return discardSchema(ids);
		case 'cut':
		case 'bid1':
		case 'bid2':
			return bidSchema(ids);
	}
}

/** The `schemaName` the provider sees. Coarse on purpose; it is a hint, not a contract. */
export function schemaNameFor(kind: AIDecisionKind): string {
	return kind === 'play' ? 'EuchrePlay' : 'EuchreBid';
}

export const SCHEMA_DESCRIPTION = 'The single legal move id this player chooses, copied verbatim from LEGAL:.';
