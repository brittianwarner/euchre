// Writes .br (brotli q11) and .gz (gzip 9) siblings for html, js, wasm and ttf files.
//   bun precompress.ts <file-or-dir>...
// PNG and WebP do not shrink, so they get none. Existing siblings are replaced.
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const EXTENSIONS = new Set(['.html', '.js', '.wasm', '.ttf']);

function* files(path: string): Generator<string> {
	if (statSync(path).isDirectory()) {
		for (const name of readdirSync(path)) yield* files(join(path, name));
	} else {
		yield path;
	}
}

let count = 0;
for (const root of process.argv.slice(2)) {
	for (const file of files(root)) {
		const dot = file.lastIndexOf('.');
		if (dot < 0 || !EXTENSIONS.has(file.slice(dot))) continue;
		const bytes = readFileSync(file);
		writeFileSync(
			`${file}.br`,
			brotliCompressSync(bytes, {
				params: {
					[constants.BROTLI_PARAM_QUALITY]: 11,
					[constants.BROTLI_PARAM_SIZE_HINT]: bytes.length
				}
			})
		);
		writeFileSync(`${file}.gz`, gzipSync(bytes, { level: 9 }));
		count++;
	}
}
console.log(`precompressed ${count} files`);
