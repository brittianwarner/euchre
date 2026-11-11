/**
 * Client-side table view model.
 *
 * Owns the public `PublicGameView` the UI renders, applies `sync` events, and
 * submits moves through the Rivet actor handle.
 */

import type { LegalMoveId, PublicGameView, SyncEvent } from '#lib/protocol/index.ts';

/** Minimal actor surface the play page injects (proxied Rivet methods). */
export interface TableActorHandle {
	readonly isConnected: boolean;
	readonly lastActionError?: unknown;
	snapshot(): Promise<PublicGameView | undefined>;
	submitMove(request: {
		moveId: LegalMoveId;
		turnId: string;
		clientMoveId: string;
	}): Promise<{ ok: true; v: number } | { ok: false; code?: string } | undefined>;
}

/**
 * Reactive store for one connected table.
 * Sync is applied from the page via {@link applySync} (onEvent at component init).
 */
export class TableStore {
	view = $state.raw<PublicGameView | null>(null);
	status = $state<'idle' | 'connecting' | 'ready' | 'error'>('idle');
	error = $state<string | null>(null);
	submitting = $state(false);

	#handle: TableActorHandle | null = null;
	#resyncPromise: Promise<void> | null = null;

	/** Remember the actor handle for snapshot / submit. */
	bind(handle: TableActorHandle): void {
		this.#handle = handle;
		this.status = handle.isConnected ? 'ready' : 'connecting';
	}

	/**
	 * The table's spoken log — what a real table would say out loud.
	 *
	 * These are engine-authored lines ("Order it up.", "I assist.", "March!",
	 * "Euchre!") plus screened persona banter. Bounded, because a long match
	 * would otherwise grow this without limit.
	 *
	 * Deliberately NOT the AI's decision rationale. A rationale is generated from
	 * that seat's full view including its own cards — one real example was
	 * "calling next from first seat with two high trump, a side ace" — so
	 * broadcasting it would hand every player the caller's holdings. Table talk is
	 * public by construction; reasoning is not.
	 */
	chat = $state<{ msgId: string; seat: number; kind: string; text: string }[]>([]);

	/** Append one spoken line, keeping only the most recent few. */
	applyChat(payload: { msgId: string; seat: number; kind: string; text: string }): void {
		if (this.chat.some((m) => m.msgId === payload.msgId)) return; // redelivery
		this.chat = [...this.chat, payload].slice(-6);
	}

	/** Apply a `sync` event from the actor (called from page-level onEvent). */
	applySync(payload: SyncEvent): void {
		if (this.view && payload.view.v < this.view.v) return;
		this.view = payload.view;
		this.status = 'ready';
		this.error = null;
	}

	/** Hard resync after connect — snap, do not animate. */
	resync(): Promise<void> {
		if (this.#resyncPromise) return this.#resyncPromise;
		this.#resyncPromise = this.#fetchSnapshot().finally(() => {
			this.#resyncPromise = null;
		});
		return this.#resyncPromise;
	}

	async #fetchSnapshot(): Promise<void> {
		const handle = this.#handle;
		if (!handle?.isConnected) return;
		let timer: ReturnType<typeof setTimeout> | undefined;
		try {
			const snap = await Promise.race([
				handle.snapshot(),
				new Promise<never>((_, reject) => {
					timer = setTimeout(() => reject(new Error('Snapshot timed out')), 4000);
				})
			]);
			if (!snap) throw new Error('Snapshot unavailable');
			if (!this.view || snap.v >= this.view.v) {
				this.view = snap;
				this.status = 'ready';
				this.error = null;
			}
		} catch {
			this.error = 'Waiting for the table to reconnect. Your game is saved.';
		} finally {
			clearTimeout(timer);
		}
	}

	/** Submit a legal move id from the current view. */
	async play(moveId: LegalMoveId): Promise<boolean> {
		const handle = this.#handle;
		const view = this.view;
		if (!handle || !view || this.submitting) return false;
		if (!view.legal.some((m) => m.id === moveId)) {
			this.error = 'That move is not legal right now.';
			return false;
		}

		this.submitting = true;
		this.error = null;
		const clientMoveId = crypto.randomUUID();
		try {
			const ack = await handle.submitMove({
				moveId,
				turnId: view.turnId,
				clientMoveId
			});
			if (!ack || ack.ok === false) {
				await this.resync();
				// A lost acknowledgement is not a rejected move. The same
				// turn nonce must still be current before offering a retry.
				if (this.view && this.view.turnId !== view.turnId) return true;
				this.error = 'The table could not confirm your move. Your game is saved. Please try again.';
				return false;
			}
			if (!this.view || this.view.v < ack.v) await this.resync();
			return true;
		} catch (err) {
			console.error('[table-store] play error', err);
			await this.resync();
			if (this.view && this.view.turnId !== view.turnId) return true;
			this.error = 'Connection interrupted. Your game is saved. Reconnect and try again.';
			return false;
		} finally {
			this.submitting = false;
		}
	}
}
