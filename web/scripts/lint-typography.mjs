import { readdir, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { checkCss, checkScript } from './typography-policy.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const ignored = new Set(['node_modules', 'dist', 'playwright-report', 'test-results', 'visual', 'scripts']);
const errors = [];
async function walk(directory) {
  for (const item of await readdir(directory, { withFileTypes: true })) {
    if (ignored.has(item.name)) continue;
    const file = `${directory}/${item.name}`;
    if (item.isDirectory()) await walk(file);
    else if (/\.(css|tsx?|jsx?)$/.test(file) && !file.endsWith('.test.ts')) {
      const source = await readFile(file, 'utf8');
      errors.push(...(file.endsWith('.css') ? checkCss(source, file) : checkScript(source, file)));
    }
  }
}
await walk(root);
if (errors.length) {
  console.error(errors.join('\n'));
  process.exitCode = 1;
} else console.log('Typography policy passed: authored font sizes use at least 1rem.');
