#!/usr/bin/env node
/**
 * Findet deutsche Anfuehrungszeichen, die mit einem ASCII-" statt mit " geschlossen werden.
 *
 * In einem "..."-String beendet das ASCII-Zeichen das Literal und bricht den Build
 * (TS1005/TS1002) — in Template-Literals und Kommentaren faellt es nur typografisch auf.
 * Beides wird hier gemeldet, damit es lokal auffaellt statt erst in der CI.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const TARGETS = ["src", "index.html"];
const EXTS = [".ts", ".tsx", ".html"];

const OPEN = "„"; // „
const CLOSE = "“"; // "

function walk(path, out = []) {
  const st = statSync(path);
  if (st.isFile()) {
    if (EXTS.some((e) => path.endsWith(e))) out.push(path);
    return out;
  }
  for (const entry of readdirSync(path)) walk(join(path, entry), out);
  return out;
}

const findings = [];

for (const target of TARGETS) {
  for (const file of walk(join(ROOT, target))) {
    const lines = readFileSync(file, "utf8").split("\n");
    lines.forEach((line, i) => {
      for (let at = line.indexOf(OPEN); at !== -1; at = line.indexOf(OPEN, at + 1)) {
        // Vom oeffnenden „ nach rechts: welches Anfuehrungszeichen kommt als naechstes?
        const rest = line.slice(at + 1);
        const next = rest.search(/[„“"]/);
        if (next !== -1 && rest[next] === '"') {
          findings.push({
            file: relative(ROOT, file),
            line: i + 1,
            col: at + 1,
            text: line.trim(),
          });
        }
      }
    });
  }
}

if (findings.length === 0) {
  console.log("check-quotes: ok");
  process.exit(0);
}

console.error(
  `\ncheck-quotes: ${findings.length} Stelle(n) mit ${OPEN}...\" statt ${OPEN}...${CLOSE}\n`,
);
for (const f of findings) {
  console.error(`  ${f.file}:${f.line}:${f.col}`);
  console.error(`    ${f.text}`);
}
console.error(
  `\nIn einem \"...\"-String beendet das ASCII-\" das Literal und bricht den Build.` +
    `\nSchliessendes Zeichen ist ${CLOSE} (U+201C).\n`,
);
process.exit(1);
