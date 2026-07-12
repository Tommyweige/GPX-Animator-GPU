import { execFileSync } from 'node:child_process';
import { copyFile, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { inject } from 'postject';

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)));
const PUBLIC = join(ROOT, 'public');
const BUILD = join(ROOT, 'build');
const DIST = join(ROOT, 'dist');
const MAIN = join(ROOT, 'packaging', 'sea-main.cjs');
const CONFIG = join(BUILD, 'sea-config.json');
const BLOB = join(BUILD, 'sea-prep.blob');
const EXE = join(DIST, 'GPX-Animator-GPU.exe');
const FUSE = 'NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2';

async function collectFiles(directory) {
  const result = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const absolute = join(directory, entry.name);
    if (entry.isDirectory()) result.push(...await collectFiles(absolute));
    else if (entry.isFile()) result.push(absolute);
  }
  return result;
}

function assetName(path) {
  return relative(PUBLIC, path).split(sep).join('/');
}

await rm(BUILD, { recursive: true, force: true });
await rm(DIST, { recursive: true, force: true });
await mkdir(BUILD, { recursive: true });
await mkdir(DIST, { recursive: true });

const publicFiles = await collectFiles(PUBLIC);
const assets = Object.fromEntries(publicFiles.map((path) => [assetName(path), path]));
const config = {
  main: MAIN,
  output: BLOB,
  disableExperimentalSEAWarning: true,
  useSnapshot: false,
  useCodeCache: false,
  assets,
};
await writeFile(CONFIG, `${JSON.stringify(config, null, 2)}\n`, 'utf8');

console.log(`Embedding ${publicFiles.length} assets...`);
execFileSync(process.execPath, ['--experimental-sea-config', CONFIG], { cwd: ROOT, stdio: 'inherit' });
await copyFile(process.execPath, EXE);

console.log('Injecting the SEA blob...');
await inject(EXE, 'NODE_SEA_BLOB', await readFile(BLOB), {
  sentinelFuse: FUSE,
  machoSegmentName: 'NODE_SEA',
});

const check = execFileSync(EXE, ['--self-test'], { cwd: DIST, encoding: 'utf8', windowsHide: true }).trim();
const result = JSON.parse(check.split(/\r?\n/).at(-1));
if (!result.sea || result.indexBytes < 1) throw new Error('SEA self-test failed');

const info = await stat(EXE);
console.log(`Built: ${EXE}`);
console.log(`Size: ${(info.size / 1024 / 1024).toFixed(1)} MB`);
