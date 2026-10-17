#!/usr/bin/env node
// Build a self-contained Node 22 worker; no Host node_modules installation.
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const require = createRequire(path.join(root, 'apps/desktop/package.json'));
const paged = process.argv.includes('--paged');
const outArg = process.argv.slice(2).find(arg => arg !== '--paged');
const { build } = await import(pathToFileURL(require.resolve('vite')).href);
await build({
  configFile: false,
  root: path.join(root, 'apps/desktop'),
  logLevel: 'warn',
  build: {
    target: 'node22',
    outDir: outArg ? path.resolve(outArg) : path.join(root, paged ? 'target/tmux-history/tmux-v2' : 'target/tmux-history'),
    emptyOutDir: false,
    minify: false,
    lib: { entry: path.join(root, paged ? 'scripts/openab-runtime/tmux-v2/tmux-history.ts' : 'scripts/openab-runtime/tmux-history.ts'), formats: ['cjs'], fileName: () => 'tmux-history.cjs' },
    rollupOptions: { external: [/^node:/] },
  },
});
