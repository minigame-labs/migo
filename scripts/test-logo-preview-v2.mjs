import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

const previewPath = resolve("brand/migo-logo-preview-v2.html");
const html = existsSync(previewPath) ? readFileSync(previewPath, "utf8") : "";
const concepts = ["d1", "d2", "d3", "d4"];
const assets = ["sugar-glider.png", "pangolin.png", "pocket-dragon.png", "manta.png"];
const checks = [
  ["preview document", /<!doctype html>/i],
  ["responsive viewport", /<meta name="viewport"/],
  ["live selected preview", /id="selected-preview"[^>]+aria-live="polite"/],
  ["recommended pangolin", /data-concept="d2"[^>]+data-recommended/],
  ["small icon test", /data-size="16"/],
  ["monochrome test", /data-treatment="mono"/],
  ["horizontal lockup", /data-layout="horizontal"/],
  ["theme control", /data-theme-choice="dark"/],
  ["selection behavior", /function selectConcept/],
];

for (const concept of concepts) {
  checks.push([`${concept} card`, new RegExp(`data-concept="${concept}"`)]);
  checks.push([`${concept} reduced symbol`, new RegExp(`id="mark-${concept}"`)]);
  checks.push([`${concept} selector`, new RegExp(`data-select="${concept}"`)]);
}

for (const asset of assets) {
  checks.push([`${asset} reference`, new RegExp(`assets/migo-v2/${asset}`)]);
  checks.push([`${asset} file`, existsSync(resolve(`brand/assets/migo-v2/${asset}`)) ? /./ : /$a/]);
}

const failures = checks.filter(([, pattern]) => !pattern.test(html)).map(([name]) => name);
if (/(?:src|href)="https?:\/\//.test(html) || /<link[^>]+rel="stylesheet"/.test(html)) {
  failures.push("external resource found");
}

if (failures.length) {
  console.error(`Migo v2 preview contract failed:\n- ${failures.join("\n- ")}`);
  process.exit(1);
}

console.log(`Migo v2 preview contract passed: ${checks.length} checks`);
