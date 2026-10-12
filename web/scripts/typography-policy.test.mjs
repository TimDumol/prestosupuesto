import { test } from 'node:test';
import assert from 'node:assert/strict';
import { checkCss, checkScript, readableSize } from './typography-policy.mjs';

test('sizes respect the browser font preference and the minimum, including fluid headings', () => {
  for (const size of ['1rem', '1.5rem', 'var(--text-body)', 'inherit', 'clamp(1.5rem, 3vw, 3rem)']) assert.ok(readableSize(size), size);
  for (const size of ['12px', '16px', '.8rem', '80%', '.8em', 'smaller', 'calc(1rem - 2px)', 'var(--tiny)', 'clamp(.8rem, 3vw, 2rem)']) assert.ok(!readableSize(size), size);
});
test('nested media queries and shorthand cannot bypass CSS checks', () => {
  assert.equal(checkCss('@media (max-width: 600px) { .label { font-size: 12px; } }', 'bad.css').length, 1);
  assert.equal(checkCss('.label { font: 12px Arial; --text-body: .8rem; }', 'bad.css').length, 2);
  assert.deepEqual(checkCss('.label { font-size: var(--text-body); font: inherit; }', 'ok.css'), []);
});
test('inline React and SVG font sizes are checked with the TypeScript parser', () => {
  assert.equal(checkScript('const x = { font: "12px Arial" };', 'bad.tsx').length, 1);
  assert.equal(checkScript('const x = { fontSize: 12 }; const y = <text fontSize={11} />;', 'bad.tsx').length, 2);
  assert.equal(checkScript('const x = { "font-size": "12px" }; const y = <text fontSize={size} />;', 'bad.tsx').length, 2);
  assert.deepEqual(checkScript('const x = { fontSize: "1rem" }; const y = <text fontSize="1.25rem" />;', 'ok.tsx'), []);
});
