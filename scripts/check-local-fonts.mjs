// Post-build check for #89: the dashboard must not fetch fonts from the
// network. Runs as the last step of `npm run build`, so CI's build step fails
// if a Google Fonts reference comes back or a bundled face drops out of dist/.
//
// This checks files, not rendering. Whether the app looks the same with the
// network off is a runtime check on a real host and is not established here.

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const REMOTE = /fonts\.googleapis\.com|fonts\.gstatic\.com/;
const FACES = [
  "IBMPlexSans-Regular",
  "IBMPlexSans-Medium",
  "IBMPlexSans-SemiBold",
  "IBMPlexMono-Regular",
  "IBMPlexMono-Medium",
];
const TEXT = /\.(html|css|js|mjs|ts|tsx|json)$/;

const failures = [];

function walk(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}

// 1. No remote font host in the page, the sources, or the built output.
const scanned = [join(root, "index.html"), ...walk(join(root, "src")), ...walk(join(root, "dist"))];
for (const file of scanned.filter((f) => TEXT.test(f))) {
  if (REMOTE.test(readFileSync(file, "utf8"))) {
    failures.push(`remote font host referenced in ${relative(root, file)}`);
  }
}

// 2. Every face is bundled as a woff2 in dist/ and the built CSS points at it.
const dist = walk(join(root, "dist"));
const css = dist.filter((f) => f.endsWith(".css")).map((f) => readFileSync(f, "utf8")).join("\n");
for (const face of FACES) {
  const emitted = dist.map((f) => f.split(/[\\/]/).pop()).find((n) => n.startsWith(`${face}-`) && n.endsWith(".woff2"));
  if (!emitted) {
    failures.push(`dist/ has no woff2 for ${face}`);
  } else if (!css.includes(emitted)) {
    failures.push(`built CSS does not reference ${emitted}`);
  }
}

// 3. The OFL license ships next to the font files.
const license = join(root, "src", "assets", "fonts", "OFL.txt");
if (!existsSync(license) || !readFileSync(license, "utf8").includes("SIL Open Font License")) {
  failures.push("src/assets/fonts/OFL.txt is missing or is not the SIL Open Font License");
}

if (failures.length > 0) {
  console.error("check-local-fonts: FAILED");
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log(`check-local-fonts: ok (${FACES.length} faces bundled, no remote font host)`);
