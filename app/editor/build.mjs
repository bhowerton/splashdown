// Bundles the editor into app/ui/vendor/editor.js, with the licences of
// what's in it beside it. Run with `pnpm build` after changing src/ or the
// dependencies; both files are committed, so building the app needs no
// JavaScript tools.

import { build } from "esbuild";
import { readFileSync, writeFileSync, readdirSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, "../ui/vendor");

const result = await build({
  entryPoints: [join(here, "src/editor.js")],
  bundle: true,
  minify: true,
  format: "iife",
  globalName: "NoteEditor",
  target: ["safari15", "chrome100"],
  outfile: join(out, "editor.js"),
  metafile: true,
  legalComments: "none",
});

// Every package that ended up in the bundle, and its licence.
const packages = new Set();
const [output] = Object.values(result.metafile.outputs);
for (const [input, { bytesInOutput }] of Object.entries(output.inputs)) {
  if (!bytesInOutput) continue;
  const m = /node_modules\/(?:\.pnpm\/[^/]+\/node_modules\/)?((?:@[^/]+\/)?[^/]+)\//.exec(input);
  if (m) packages.add(m[1]);
}
const sections = [...packages].sort().map((name) => {
  const dir = join(here, "node_modules", name);
  const real = existsSync(dir) ? dir : findNested(name);
  const pkg = JSON.parse(readFileSync(join(real, "package.json"), "utf8"));
  const file = readdirSync(real).find((f) => /^licen[cs]e/i.test(f));
  const text = file ? readFileSync(join(real, file), "utf8").trim() : `License: ${pkg.license}`;
  return `${name} ${pkg.version} (${pkg.license})\n\n${text}`;
});
writeFileSync(
  join(out, "editor.LICENSES.txt"),
  `The Notes editor (editor.js) bundles these packages.\n\n${sections.join("\n\n" + "-".repeat(72) + "\n\n")}\n`,
);
console.log(`editor.js: ${packages.size} packages`);

/** A package pnpm installed only as another one's dependency. */
function findNested(name) {
  const store = join(here, "node_modules/.pnpm");
  for (const entry of readdirSync(store)) {
    const dir = join(store, entry, "node_modules", name);
    if (existsSync(join(dir, "package.json"))) return dir;
  }
  throw new Error(`can't find ${name}`);
}
