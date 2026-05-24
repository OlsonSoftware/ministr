// Copy asciinema cast files from ../assets into public/ so the hero
// player + workflow-comparison can fetch them at runtime.
//
// Runs from `predev` + `prebuild`. Pure Node (no shell) so it works
// identically on Windows `cmd.exe`, PowerShell, bash, etc. — the old
// shell one-liner failed on Windows because cmd.exe's `mkdir` doesn't
// understand `-p` and errors when the directory already exists.

import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const publicDir = join(here, '..', 'public');
const assetsDir = join(here, '..', '..', 'assets');

mkdirSync(publicDir, { recursive: true });

const casts = [
  { name: 'launch.cast', required: true },
  { name: 'launch-baseline.cast', required: false },
];

for (const { name, required } of casts) {
  const src = join(assetsDir, name);
  const dst = join(publicDir, name);
  if (existsSync(src)) {
    copyFileSync(src, dst);
  } else if (required) {
    // eslint-disable-next-line no-console
    console.warn(
      `warn: assets/${name} not yet recorded — hero player will 404 at runtime`,
    );
  }
}
