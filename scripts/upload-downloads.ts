// Upload built desktop downloads to the Railway bucket (called by release-desktop.sh --upload).
//
//   railway bucket credentials -b euchre-downloads --json | bun run scripts/upload-downloads.ts <downloads-dir>
//
// stdin: JSON with accessKeyId, secretAccessKey, endpoint, bucketName, region. Never printed.
// Keys: files/<file> (immutable, one year), then manifest.json last (no-cache).
// A file whose object already exists with the same size is skipped. Afterwards every
// object's size is read back and compared with the manifest.
import { S3Client } from "bun";
import { join } from "node:path";

interface Credentials {
  accessKeyId: string;
  secretAccessKey: string;
  endpoint: string;
  bucketName: string;
  region: string;
}

interface ManifestEntry {
  file: string;
  bytes: number;
  sha256: string;
}

interface Manifest {
  version: string;
  platforms: Record<string, ManifestEntry>;
}

const USAGE = "usage: railway bucket credentials -b euchre-downloads --json | bun run scripts/upload-downloads.ts <downloads-dir>";

const CONTENT_TYPES: Record<string, string> = {
  ".dmg": "application/x-apple-diskimage",
  ".zip": "application/zip",
  ".appimage": "application/octet-stream",
};

function contentType(file: string): string {
  const dot = file.lastIndexOf(".");
  return CONTENT_TYPES[dot < 0 ? "" : file.slice(dot).toLowerCase()] ?? "application/octet-stream";
}

function fail(message: string): never {
  console.error(`upload-downloads: ${message}`);
  process.exit(1);
}

async function main(): Promise<void> {
  const [dir] = process.argv.slice(2).filter((a) => a !== "--help" && a !== "-h");
  if (process.argv.includes("--help") || process.argv.includes("-h")) {
    console.log(USAGE);
    return;
  }
  if (!dir) fail(USAGE);

  let creds: Credentials;
  try {
    creds = JSON.parse(await Bun.stdin.text()) as Credentials;
  } catch {
    return fail("stdin is not credentials JSON");
  }
  for (const k of ["accessKeyId", "secretAccessKey", "endpoint", "bucketName", "region"] as const) {
    if (!creds[k] || typeof creds[k] !== "string") fail(`credentials are missing ${k}`);
  }

  const manifestFile = Bun.file(join(dir, "manifest.json"));
  if (!(await manifestFile.exists())) fail(`no manifest.json in ${dir}`);
  const manifest = JSON.parse(await manifestFile.text()) as Manifest;
  const entries = Object.values(manifest.platforms);
  if (entries.length === 0) fail("manifest lists no files");

  // Check the files on disk against the manifest before sending anything.
  for (const e of entries) {
    const f = Bun.file(join(dir, e.file));
    if (!(await f.exists()) || f.size !== e.bytes) fail(`${e.file} does not match the manifest`);
    const hasher = new Bun.CryptoHasher("sha256");
    for await (const chunk of f.stream()) hasher.update(chunk);
    if (hasher.digest("hex") !== e.sha256) fail(`${e.file} sha256 does not match the manifest`);
  }

  const s3 = new S3Client({
    accessKeyId: creds.accessKeyId,
    secretAccessKey: creds.secretAccessKey,
    endpoint: creds.endpoint,
    bucket: creds.bucketName,
    region: creds.region,
    // Path-style. Bun's virtualHostedStyle wants the bucket already in the
    // endpoint host; with the bare Railway endpoint it answers NoSuchBucket.
  });

  for (const e of entries) {
    const key = `files/${e.file}`;
    const obj = s3.file(key);
    const existing = (await obj.exists()) ? (await obj.stat()).size : -1;
    if (existing === e.bytes) {
      console.log(`skip   ${key}  ${e.bytes} bytes  ${e.sha256}`);
      continue;
    }
    await s3.write(key, Bun.file(join(dir, e.file)), {
      type: contentType(e.file),
      headers: { "Cache-Control": "public, max-age=31536000, immutable" },
    });
    console.log(`upload ${key}  ${e.bytes} bytes  ${e.sha256}`);
  }

  // The manifest goes last so it never names a file that is not there yet.
  await s3.write("manifest.json", await manifestFile.text(), {
    type: "application/json",
    headers: { "Cache-Control": "no-cache" },
  });
  console.log(`upload manifest.json  version ${manifest.version}`);

  for (const e of entries) {
    const size = (await s3.file(`files/${e.file}`).stat()).size;
    if (size !== e.bytes) fail(`files/${e.file} is ${size} bytes in the bucket, manifest says ${e.bytes}`);
    console.log(`verify files/${e.file}  ${size} bytes ok`);
  }
}

await main();
