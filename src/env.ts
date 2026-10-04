import { defineEnvVars } from '@sveltejs/kit/env';

// Optional server-only configuration; deployment boundaries validate required values.
const optional = { schema: (value: string | undefined) => value };
export const variables = defineEnvVars({
	ALLOWED_ORIGINS: optional,
	AUTH_SECRET: optional,
	BETTER_AUTH_SECRET: optional,
	BETTER_AUTH_URL: optional,
	DATABASE_URL: optional,
	EMAIL_FROM: optional,
	EUCHRE_ACTOR_JWT_SECRET: optional,
	PUBLIC_APP_URL: optional,
	RESEND_API_KEY: optional,
	RIVET_ENDPOINT: optional,
	RIVET_NAMESPACE: optional,
	RIVET_TOKEN: optional,
	RIVET_PUBLIC_ENDPOINT: optional,
	VERCEL_PROJECT_PRODUCTION_URL: optional,
	VERCEL_URL: optional
});
