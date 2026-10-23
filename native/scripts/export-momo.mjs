// Reproduce the native rest asset from Kivio's existing, deterministic simulation.
// Run from repository root: node native/scripts/export-momo.mjs
import { build } from 'esbuild';
import { writeFileSync } from 'node:fs';

const result = await build({
  entryPoints: ['src/chat/kivioBlobSim.ts'],
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
});
const { BLOB_REST } = await import(
  `data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`
);
const paint = BLOB_REST;
writeFileSync('native/assets/momo.svg',
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 240"><g transform="${paint.rig}"><path d="${paint.bodyD}" fill="${paint.fill}"/>${paint.eyes.map(eye => `<path d="${eye.d}" transform="${eye.transform}" fill="#f3efe6"/>`).join('')}</g></svg>`);
