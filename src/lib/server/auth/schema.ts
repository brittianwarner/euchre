/**
 * Better Auth's tables, as a Drizzle **SQLite** schema, bound at runtime to the
 * `authStore` actor's `c.db` (docs/06 §4.3, §5.3).
 *
 * There is no Postgres in this project and no `DATABASE_URL`. This schema is
 * handed to the *stock* `drizzleAdapter` from `better-auth/adapters/drizzle`,
 * so none of Better Auth's where-clause, sort, pagination or field-mapping
 * semantics are reimplemented here — docs/06 §5.2 rejects candidate (a1) for
 * exactly that reason.
 *
 * ## Field shapes
 *
 * The columns below mirror `getAuthTables()` in `@better-auth/core` for the
 * plugin set configured in `./auth.ts` (core + `magicLink` + `bearer` + the
 * database-backed rate limiter). `magicLink` and `bearer` add no tables of their
 * own; magic links are rows in `verification`.
 *
 * - `date` fields are `integer({ mode: 'timestamp_ms' })`. Millisecond precision
 *   matters: a magic link lives ten minutes and a rate-limit window sixty
 *   seconds, and second-granularity rounding on both ends of a comparison is a
 *   real off-by-one.
 * - `boolean` fields are `integer({ mode: 'boolean' })`.
 * - Every id is a text primary key. Better Auth generates ids itself; we do not
 *   opt into `generateId: 'serial'`.
 *
 * ## Where the DDL lives
 *
 * Drizzle schema objects describe *shape*, not migrations. The actual
 * `CREATE TABLE` statements are in `./migrations.ts` and run from the actor's
 * `onMigrate`. Both files must be edited together; `./migrations.ts` says so at
 * the top.
 *
 * The column helpers are re-exported by `rivetkit/db/drizzle`, so importing them
 * from there rather than from `drizzle-orm/sqlite-core` guarantees this schema
 * is built against the exact Drizzle that `c.db` speaks.
 */

import { index, integer, sqliteTable, text } from 'rivetkit/db/drizzle';

/** Better Auth `user`. */
export const user = sqliteTable('user', {
	id: text('id').primaryKey(),
	name: text('name').notNull(),
	email: text('email').notNull().unique(),
	emailVerified: integer('emailVerified', { mode: 'boolean' }).notNull().default(false),
	image: text('image'),
	createdAt: integer('createdAt', { mode: 'timestamp_ms' }).notNull(),
	updatedAt: integer('updatedAt', { mode: 'timestamp_ms' }).notNull()
});

/**
 * Better Auth `session`.
 *
 * `token` is the credential the browser holds in its cookie. It is also what
 * `verifySessionToken()` accepts on its slow path, so this table is on the
 * authorization path for every Rivet connection that does not present a minted
 * actor token.
 */
export const session = sqliteTable(
	'session',
	{
		id: text('id').primaryKey(),
		expiresAt: integer('expiresAt', { mode: 'timestamp_ms' }).notNull(),
		token: text('token').notNull().unique(),
		createdAt: integer('createdAt', { mode: 'timestamp_ms' }).notNull(),
		updatedAt: integer('updatedAt', { mode: 'timestamp_ms' }).notNull(),
		ipAddress: text('ipAddress'),
		userAgent: text('userAgent'),
		userId: text('userId')
			.notNull()
			.references(() => user.id, { onDelete: 'cascade' })
	},
	(table) => [index('session_userId_idx').on(table.userId)]
);

/**
 * Better Auth `account`.
 *
 * Unused today — this app has no social providers and no password auth, only
 * magic links. The table still exists because Better Auth's core queries it
 * during sign-in and account linking, and a missing model is a hard adapter
 * error rather than a graceful no-op.
 */
export const account = sqliteTable(
	'account',
	{
		id: text('id').primaryKey(),
		accountId: text('accountId').notNull(),
		providerId: text('providerId').notNull(),
		userId: text('userId')
			.notNull()
			.references(() => user.id, { onDelete: 'cascade' }),
		accessToken: text('accessToken'),
		refreshToken: text('refreshToken'),
		idToken: text('idToken'),
		accessTokenExpiresAt: integer('accessTokenExpiresAt', { mode: 'timestamp_ms' }),
		refreshTokenExpiresAt: integer('refreshTokenExpiresAt', { mode: 'timestamp_ms' }),
		scope: text('scope'),
		password: text('password'),
		createdAt: integer('createdAt', { mode: 'timestamp_ms' }).notNull(),
		updatedAt: integer('updatedAt', { mode: 'timestamp_ms' }).notNull()
	},
	(table) => [index('account_userId_idx').on(table.userId)]
);

/**
 * Better Auth `verification` — where magic-link tokens live until they are
 * consumed. A token is deleted the first time it verifies, so a replayed link
 * mints nothing.
 */
export const verification = sqliteTable(
	'verification',
	{
		id: text('id').primaryKey(),
		identifier: text('identifier').notNull(),
		value: text('value').notNull(),
		expiresAt: integer('expiresAt', { mode: 'timestamp_ms' }).notNull(),
		createdAt: integer('createdAt', { mode: 'timestamp_ms' }).notNull(),
		updatedAt: integer('updatedAt', { mode: 'timestamp_ms' }).notNull()
	},
	(table) => [index('verification_identifier_idx').on(table.identifier)]
);

/**
 * Better Auth `rateLimit`, used because `./auth.ts` sets
 * `rateLimit.storage: 'database'`.
 *
 * Memory storage would be wrong twice over: the counters would live in one
 * function instance's heap (docs/06 §5.2 candidate (c)), and even inside the
 * singleton actor they would reset on every serverless migration — which is
 * exactly when an attacker's burst would sail through. Rows here are durable
 * and migrate with the actor.
 *
 * **Retention:** Better Auth resets a row's window in place rather than
 * inserting, so the table is bounded by *distinct rate-limit keys*
 * (`{ip}-{path}`) rather than by request volume. `./migrations.ts` does not
 * garbage-collect it; if it is ever measured to grow, a `lastRequest` sweep in
 * the actor's `onWake` is the fix.
 */
export const rateLimit = sqliteTable('rateLimit', {
	id: text('id').primaryKey(),
	key: text('key').notNull().unique(),
	count: integer('count').notNull(),
	lastRequest: integer('lastRequest').notNull()
});

/**
 * The object handed to `drizzleAdapter(db, { schema })`. The keys are Better
 * Auth *model* names and must stay exactly these strings.
 */
export const authSchema = { user, session, account, verification, rateLimit };

export type AuthSchema = typeof authSchema;
