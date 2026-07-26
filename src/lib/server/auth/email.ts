/**
 * Magic-link delivery.
 *
 * Two delivery paths, chosen by whether `RESEND_API_KEY` is configured:
 *
 *  - **Resend** (`resend@6.18.0`) when the key is present.
 *  - **The server console** when it is not *and* we are in dev. This is a hard
 *    requirement, not a nicety: a contributor must be able to clone the repo,
 *    `bun run dev` with an empty `.env`, and sign in.
 *
 * In a deployed environment with no key, neither path fires and the request
 * fails loudly. Printing a magic link to a production log is an account
 * takeover for anyone who can read logs, so `canLogMagicLink()` is gated on
 * `dev` and never on "the key happens to be missing".
 */

import { APIError } from 'better-auth/api';
import { Resend } from 'resend';
import { canLogMagicLink, getEmailFrom, getResendApiKey, MAGIC_LINK_EXPIRES_IN_SECONDS } from './env';

/** What the `magicLink` plugin hands us. */
export interface MagicLinkMail {
	/** The recipient. Already normalised by Better Auth. */
	readonly email: string;
	/**
	 * The one-time sign-in URL, `${baseURL}/api/auth/magic-link/verify?token=…`.
	 *
	 * **Secret.** Anything holding this URL can become the user. It is never
	 * logged outside dev and never returned in an HTTP response.
	 */
	readonly url: string;
}

/**
 * Memoised Resend client. Built lazily so that importing this module during
 * `vite build` — which evaluates server modules with an empty environment —
 * does not require the key.
 */
let resend: Resend | undefined;
let resendKey: string | undefined;

function getResend(apiKey: string): Resend {
	if (!resend || resendKey !== apiKey) {
		resend = new Resend(apiKey);
		resendKey = apiKey;
	}
	return resend;
}

/**
 * Send (or, in dev, print) a magic link.
 *
 * Throws `APIError` on failure so Better Auth answers the sign-in request with
 * a 500 and a fixed message rather than a stack trace. The *cause* is logged
 * server-side; the client is told only that delivery failed.
 */
export async function sendMagicLinkEmail(mail: MagicLinkMail): Promise<void> {
	const apiKey = getResendApiKey();

	if (!apiKey) {
		if (canLogMagicLink()) {
			logMagicLinkToConsole(mail);
			return;
		}
		console.error('[auth] RESEND_API_KEY is unset; refusing to deliver a magic link.');
		throw new APIError('INTERNAL_SERVER_ERROR', {
			message: 'Email delivery is not configured.'
		});
	}

	const minutes = Math.round(MAGIC_LINK_EXPIRES_IN_SECONDS / 60);

	// The Resend SDK resolves with `{ data, error }` and does NOT throw on an
	// API error — a bare `await` with no branch silently swallows every failure
	// and reports success to the user. The try/catch around it is for transport
	// faults (DNS, socket, abort), which *do* throw.
	let result: Awaited<ReturnType<Resend['emails']['send']>>;
	try {
		result = await getResend(apiKey).emails.send({
			from: getEmailFrom(),
			to: [mail.email],
			subject: 'Your Euchre sign-in link',
			text: plainTextBody(mail.url, minutes),
			html: htmlBody(mail.url, minutes)
		});
	} catch (cause) {
		console.error('[auth] magic-link delivery threw', {
			name: cause instanceof Error ? cause.name : 'unknown',
			message: cause instanceof Error ? cause.message : String(cause)
		});
		throw new APIError('INTERNAL_SERVER_ERROR', {
			message: 'Could not send the sign-in email. Please try again.'
		});
	}

	if (result.error) {
		// `result.error` is Resend's own `{ name, message }`. It never contains
		// the link, but it can contain the recipient address, so it is logged as
		// two named fields rather than spread.
		console.error('[auth] magic-link delivery failed', {
			name: result.error.name,
			message: result.error.message
		});
		throw new APIError('INTERNAL_SERVER_ERROR', {
			message: 'Could not send the sign-in email. Please try again.'
		});
	}

	// Log the message id, never the link and never the address.
	console.info('[auth] magic link sent', { id: result.data?.id ?? null });
}

/**
 * Dev-only console delivery.
 *
 * Formatted as a block rather than a one-liner because the whole point is that
 * a human finds it instantly in a noisy Vite log.
 */
function logMagicLinkToConsole(mail: MagicLinkMail): void {
	const line = '─'.repeat(72);
	console.info(
		[
			'',
			line,
			'  MAGIC LINK (dev only — RESEND_API_KEY is unset, no email was sent)',
			`  to:  ${mail.email}`,
			`  url: ${mail.url}`,
			line,
			''
		].join('\n')
	);
}

function plainTextBody(url: string, minutes: number): string {
	return [
		'Sign in to Euchre',
		'',
		'Open this link to sign in:',
		url,
		'',
		`The link works once and expires in ${minutes} minutes.`,
		'If you did not ask to sign in, ignore this email.'
	].join('\n');
}

/**
 * Deliberately minimal HTML: one heading, one anchor, one fallback URL.
 *
 * `url` is produced by Better Auth from our own `baseURL`, so it is not
 * attacker-controlled, but it is still HTML-escaped on the way into both the
 * attribute and the text node — an unescaped `"` in a URL is how an anchor
 * turns into an injection point.
 */
function htmlBody(url: string, minutes: number): string {
	const safe = escapeHtml(url);
	return `<!doctype html>
<html lang="en"><body style="margin:0;padding:24px;background:#0f1115;color:#e8e6e3;font:16px/1.5 system-ui,sans-serif">
<h1 style="margin:0 0 16px;font-size:20px">Sign in to Euchre</h1>
<p style="margin:0 0 24px">Click the button below to sign in.</p>
<p style="margin:0 0 24px">
  <a href="${safe}" style="display:inline-block;padding:12px 20px;border-radius:8px;background:#2f6f4f;color:#fff;text-decoration:none;font-weight:600">Sign in</a>
</p>
<p style="margin:0 0 8px;font-size:13px;opacity:.75">Or paste this into your browser:</p>
<p style="margin:0 0 24px;font-size:13px;word-break:break-all"><a href="${safe}" style="color:#8fd0aa">${safe}</a></p>
<p style="margin:0;font-size:13px;opacity:.6">The link works once and expires in ${minutes} minutes. If you did not ask to sign in, ignore this email.</p>
</body></html>`;
}

function escapeHtml(value: string): string {
	return value
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;')
		.replace(/'/g, '&#39;');
}
