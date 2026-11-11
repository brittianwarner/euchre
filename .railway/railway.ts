import { bucket, defineRailway, image, preserve, project, ref, service, volume } from "railway/iac";

// Euchre on Railway: the Rust server (`euchre`, root Dockerfile) in front of a
// private Rivet engine 3.1.0 (`rivet-engine`). Runbook: docs/DEPLOY.md.
//
// Omitting a service, volume or variable here deletes it on apply. Secrets are
// preserve(): Railway keeps the value it holds and this file never sees it.
// A human reviews `railway config plan` and runs `railway config apply`.

export default defineRailway(() => {
  // Kept for rollback: the 2.3 engine's RocksDB still sits at the volume root.
  // Engine 3 writes only under /data/v3 (fresh storage; D10, TS tables are not
  // migrated). One volume per service, so it stays on this one.
  const rivetData = volume("rivet-data", {
    alerts: { usage: { "100": {}, "80": {}, "95": {} } },
    allowOnlineResize: true,
    region: "us-east4-eqdc4a",
    sizeMB: 10000,
  });

  // Desktop downloads (Mac, Windows, Linux) behind euchre.sh/download/*: the
  // server redirects to short-lived presigned URLs. Bucket egress is free.
  // Release script: scripts/release-desktop.sh (docs/DEPLOY.md §9).
  const downloads = bucket("euchre-downloads", { region: "iad" });

  // Rivet engine object storage (private; never presigned). Engine state and the
  // downloads never share a bucket or a credential.
  const rivetObjects = bucket("rivet-objects", { region: "iad" });

  const rivetEngine = service("rivet-engine", {
    source: image(
      "rivetdev/engine:3.1.0@sha256:fd679281d8a357d20081a6639933d1fcaecdc6fc087604f7a05ee0905a8db731",
    ),
    // The image's entrypoint is the engine (no `start` subcommand in 3.x).
    // Engine 3 refuses a log directory that does not exist yet.
    start: '/bin/sh -c "mkdir -p /data/v31b/log && exec /usr/bin/rivet-engine"',
    healthcheck: "/health",
    healthcheckTimeout: 120,
    replicas: { "us-east4-eqdc4a": 1 },
    deploy: {
      drainingSeconds: 120,
      limitOverride: { containers: { cpu: 2, memoryBytes: 2000000000 } },
      overlapSeconds: 0,
      restartPolicyType: "ALWAYS",
    },
    volumeMounts: { "/data": rivetData },
    env: {
      // Guard: private API, gateway and /health. Engine 3 binds the API peer
      // (6421) to loopback, so the healthcheck must use the guard port.
      // Never change this port or the topology on used storage: engine 3 ties
      // its token signing ring to them and would refuse every gateway token.
      PORT: "6420",
      RIVET__AUTH__ADMIN_TOKEN: preserve(),
      // Single node: the write log lives on the volume, cold storage in the
      // rivet-objects bucket (same pattern as Layerr's engine). Fresh start on
      // 2026-11-11: the old /data/v3 (log + objects) is left untouched for
      // rollback and never read. The S3 client has no addressing-style key, so
      // it reads AWS_VIRTUAL_HOSTED_STYLE_REQUEST and the endpoint carries the
      // bucket host.
      AWS_VIRTUAL_HOSTED_STYLE_REQUEST: "true",
      RIVET__STORAGE__LOG_DIRECTORY: "/data/v31b/log",
      RIVET__OBJECT_STORAGE__PRIMARY__S3__BUCKET: ref(rivetObjects, "BUCKET"),
      RIVET__OBJECT_STORAGE__PRIMARY__S3__ENDPOINT: "https://${{rivet-objects.BUCKET}}.t3.storageapi.dev",
      RIVET__OBJECT_STORAGE__PRIMARY__S3__PREFIX: "euchre-v31b/",
      RIVET__OBJECT_STORAGE__PRIMARY__S3__REGION: ref(rivetObjects, "REGION"),
      RIVET__OBJECT_STORAGE__PRIMARY__S3__ACCESS_KEY_ID: ref(rivetObjects, "ACCESS_KEY_ID"),
      RIVET__OBJECT_STORAGE__PRIMARY__S3__SECRET_ACCESS_KEY: ref(rivetObjects, "SECRET_ACCESS_KEY"),
      RIVET__STORAGE__BALANCING__ENABLED: "false",
      RIVET__STORAGE__AUTO_PROVISION_PARTITIONS: "1",
      RIVET__STORAGE__LOG_SHARD__MAX_DISK_BYTES: "4294967296",
      // Unset, the engine sizes a local cold-read cache from free disk and
      // refuses to boot: the 10 GB volume minus its 4 GiB reserve and the 4 GiB
      // log leaves none. 0 sends cold reads straight to the bucket.
      RIVET__STORAGE__COLD_STORE__DISK_CACHE_BYTES: "0",
      RIVET__TELEMETRY__ENABLED: "false",
      // Finish a graceful stop inside Railway's 120 s drain (defaults run to
      // 9 min 50 s). Engine rules: force > stop + teardown, and stop > the
      // 60 s cold-write age plus 10 % jitter.
      RIVET__STORAGE__STOP_TIMEOUT_MS: "80000",
      RIVET__STORAGE__STOP_TEARDOWN_MS: "20000",
      RIVET__RUNTIME__GUARD_SHUTDOWN_DURATION: "60",
      RIVET__RUNTIME__FORCE_SHUTDOWN_DURATION: "110",
    },
  });

  const euchre = service("euchre", {
    build: { buildEnvironment: "V3", builder: "DOCKERFILE", dockerfilePath: "Dockerfile" },
    healthcheck: "/health",
    healthcheckTimeout: 120,
    replicas: { "us-east4-eqdc4a": 1 },
    deploy: {
      drainingSeconds: 120,
      limitOverride: { containers: { cpu: 2, memoryBytes: 2000000000 } },
      overlapSeconds: 60,
      restartPolicyType: "ALWAYS",
    },
    // Names ServerConfig::from_env reads (crates/euchre-server-core/src/config.rs).
    env: {
      NODE_ENV: "production",
      PORT: "3000",
      // Reached only through the Railway edge, which appends X-Forwarded-For.
      EUCHRE_TRUST_FORWARDED_FOR: "1",
      APP_URL: preserve(),
      ALLOWED_ORIGINS: preserve(),
      RIVET_ENDPOINT: preserve(),
      RIVET_NAMESPACE: preserve(),
      RIVET_POOL: preserve(),
      RIVET_TOKEN: preserve(),
      EUCHRE_ACTOR_JWT_SECRET: preserve(),
      EUCHRE_INTERNAL_TOKEN: preserve(),
      // New for the Rust server: set it in Railway before the plan (DEPLOY.md).
      EUCHRE_IDENTITY_SECRET: preserve(),
      OPENROUTER_API_KEY: preserve(),
      // Desktop downloads bucket (crates/euchre-http/src/downloads.rs bucket
      // mode): manifest.json + files/<file>, served by presigned redirects.
      EUCHRE_DOWNLOADS_S3_BUCKET: ref(downloads, "BUCKET"),
      EUCHRE_DOWNLOADS_S3_ENDPOINT: ref(downloads, "ENDPOINT"),
      EUCHRE_DOWNLOADS_S3_REGION: ref(downloads, "REGION"),
      EUCHRE_DOWNLOADS_S3_ACCESS_KEY_ID: ref(downloads, "ACCESS_KEY_ID"),
      EUCHRE_DOWNLOADS_S3_SECRET_ACCESS_KEY: ref(downloads, "SECRET_ACCESS_KEY"),
    },
  });

  return project("euchre", { resources: [rivetEngine, euchre, rivetData, downloads, rivetObjects] });
});
