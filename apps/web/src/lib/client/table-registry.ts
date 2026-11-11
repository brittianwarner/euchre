/**
 * Type-only description of the `euchreTable` actor for the typed Rivet client.
 *
 * The actor itself lives in the Rust server (`crates/euchre-table`), so the
 * browser has no TypeScript registry to infer from. This file declares just the
 * surface the browser uses: the connect params, the five events, and the four
 * connection-reachable actions the play page calls. Nothing here runs; every
 * import is erased at build time.
 */
import type { ActorDefinition, Registry, event } from 'rivetkit';
import type { CompletedHandReview } from '#lib/protocol/hand-review.ts';
import type {
	MoveAck,
	PublicGameView,
	SubmitMoveRequest,
	TableConnectParams,
	TableEventMap
} from '#lib/protocol/index.ts';

type EventOf<T> = ReturnType<typeof event<T>>;

type TableEvents = { [K in keyof TableEventMap]: EventOf<TableEventMap[K]> };

/** `reviewTrick` arguments: toggle trick review, or release the current trick. */
export type ReviewTrickRequest =
	{ action: 'set'; enabled: boolean } | { action: 'continue'; turnId: string };

/** The actions a player connection may call. The first parameter is the actor context. */
type TableActions = {
	snapshot: (c: unknown) => PublicGameView;
	submitMove: (c: unknown, request: SubmitMoveRequest) => MoveAck;
	reviewTrick: (c: unknown, request: ReviewTrickRequest) => { queued: true };
	getHandReview: (c: unknown, handNo: number) => CompletedHandReview | null;
};

/** State, vars, input and db are server-only, so they stay `any` here. */
type EuchreTableDefinition = ActorDefinition<
	any,
	TableConnectParams,
	any,
	any,
	any,
	any,
	TableEvents,
	Record<never, never>,
	TableActions
>;

/** The registry shape the Rust server serves, as the client sees it. */
export type TableRegistry = Registry<{ euchreTable: EuchreTableDefinition }>;
