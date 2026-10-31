// Run one registered synthetic browser regression with a lock-matched package.
import { accessSync, constants, readFileSync, realpathSync, statSync } from 'node:fs';
import { dirname, isAbsolute, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';

const scripts = new Set([
  'test-session-refresh-focus.mjs',
  'test-task-detail-navigation.mjs',
  'test-task-reader-leave.mjs',
  'test-notify-latency-ui.mjs',
  'test-update-panel.mjs',
  'test-connections-page.mjs',
  'test-provider-page.mjs',
  'test-reduced-motion.mjs',
  'test-task-search.mjs',
  'test-task-dependencies.mjs',
  'test-chief-resources.mjs',
  'test-keyboard-navigation.mjs',
]);
const name = process.argv[2];
const override = process.env.TEST_BROWSER_MODULE;
const chromiumOverride = process.env.TEST_CHROMIUM_PATH;
if (!scripts.has(name) || process.argv.length !== 3
    || (override && !isAbsolute(override))
    || (chromiumOverride && !isAbsolute(chromiumOverride))) {
  console.error('Choose one registered browser regression; runtime overrides must be absolute paths.');
  process.exit(2);
}

function blocked(message) {
  console.error(message);
  process.exit(3);
}

let modulePath;
let chromiumPath;
let prerequisite = 'root package-lock.json';
try {
  const lock = JSON.parse(readFileSync(new URL('../../package-lock.json', import.meta.url), 'utf8'));
  const pinned = lock.packages?.['node_modules/playwright']?.version;
  if (typeof pinned !== 'string' || !pinned) {
    blocked('The root lockfile must pin playwright before the browser gate can run.');
  }
  // Follow directory/entry symlinks once and validate the actual target's sibling
  // package.json. Metadata symlinks are rejected: another directory cannot vouch
  // for this entry. External packages are supported, but must match the root lock.
  prerequisite = 'actual browser module/package.json';
  modulePath = realpathSync(override
    || fileURLToPath(new URL('../../node_modules/playwright/index.mjs', import.meta.url)));
  if (!statSync(modulePath).isFile()) blocked('Browser module must be a regular file.');
  const metadataPath = join(dirname(modulePath), 'package.json');
  if (realpathSync(metadataPath) !== metadataPath) {
    blocked('Browser module package.json must reside beside the canonical module entry.');
  }
  const installed = JSON.parse(readFileSync(metadataPath, 'utf8'));
  if (installed.name !== 'playwright' || installed.version !== pinned) {
    blocked('Actual browser package name/version does not match playwright in the root lock; run npm ci or select a matching external package.');
  }
  // No module code executes until its own canonical package identity is checked.
  const { chromium } = await import(pathToFileURL(modulePath).href);
  if (!chromium || typeof chromium.executablePath !== 'function') {
    blocked('Browser module does not expose the Chromium runtime.');
  }
  prerequisite = 'Chromium executable';
  chromiumPath = realpathSync(chromiumOverride || chromium.executablePath());
  if (!statSync(chromiumPath).isFile()) blocked('Chromium executable must be a regular file.');
  accessSync(chromiumPath, constants.X_OK);
} catch {
  // Do not echo dependency exceptions: they may contain unrelated local data.
  blocked(`Browser prerequisite unavailable or invalid: ${prerequisite}; npm ci then npx --no-install playwright install chromium.`);
}
if (chromiumOverride) {
  console.error('Chromium selection: explicit TEST_CHROMIUM_PATH (not version-pinned); Playwright package matches the root lock.');
} else {
  console.error('Chromium selection: validated Playwright default executable from the lock-matched package.');
}

const testPath = fileURLToPath(new URL(`../${name}`, import.meta.url));
const result = spawnSync(process.execPath, [testPath], {
  env: { ...process.env, TEST_BROWSER_MODULE: modulePath, TEST_CHROMIUM_PATH: chromiumPath },
  stdio: 'inherit',
});
if (result.error || result.signal || result.status !== 0) {
  console.error('Browser regression failed or did not complete.');
  process.exit(1);
}
process.exit(0);
