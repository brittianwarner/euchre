import { defineRailway, image, preserve, project, service, volume } from "railway/iac";

export default defineRailway(() => {
  const rivetData = volume("rivet-data", { alerts: { usage: { "100": {}, "80": {}, "95": {} } }, allowOnlineResize: true, region: "us-east4-eqdc4a", sizeMB: 10000 });
  const rivetEngine = service("rivet-engine", {
    source: image("rivetdev/engine:2.3.23@sha256:e6d1cbfe85847bc7c3c6ed7e5b3dc6cf9aaed32e8e42d6778ac9423bd928f2bb"),
    healthcheck: "/health",
    healthcheckTimeout: 120,
    replicas: { "us-east4-eqdc4a": 1 },
    deploy: { drainingSeconds: 120, limitOverride: { containers: { cpu: 2, memoryBytes: 2000000000 } }, overlapSeconds: 0, restartPolicyType: "ALWAYS" },
    volumeMounts: { "/data": rivetData },
    env: { PORT: preserve(), RIVET__AUTH__ADMIN_TOKEN: preserve(), RIVET__FEATURES__GUARD_GATEWAY_V3__MODE: preserve(), RIVET__FEATURES__GUARD_GATEWAY_V3__PERCENTAGE: preserve(), RIVET__FILE_SYSTEM__PATH: preserve() },
  });
  const euchre = service("euchre", {
    build: { buildEnvironment: "V3", builder: "DOCKERFILE", dockerfilePath: "Dockerfile" },
    healthcheck: "/health",
    healthcheckTimeout: 120,
    replicas: { "us-east4-eqdc4a": 1 },
    deploy: { drainingSeconds: 120, limitOverride: { containers: { cpu: 2, memoryBytes: 2000000000 } }, overlapSeconds: 60, restartPolicyType: "ALWAYS" },
    env: { ALLOWED_ORIGINS: preserve(), APP_URL: preserve(), EUCHRE_ACTOR_JWT_SECRET: preserve(), EUCHRE_INTERNAL_TOKEN: preserve(), NODE_ENV: preserve(), OPENROUTER_API_KEY: preserve(), ORIGIN: preserve(), PORT: preserve(), RIVETKIT_RUNTIME_MODE: preserve(), RIVET_ENDPOINT: preserve(), RIVET_ENGINE_SELFHOSTED: preserve(), RIVET_EXPOSE_ERRORS: preserve(), RIVET_INSPECTOR_DISABLE: preserve(), RIVET_LOG_LEVEL: preserve(), RIVET_NAMESPACE: preserve(), RIVET_POOL: preserve(), RIVET_PUBLIC_ENDPOINT: preserve(), RIVET_TOKEN: preserve() },
  });

  return project("euchre", {
    resources: [rivetEngine, euchre, rivetData],
  });
});
