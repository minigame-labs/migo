import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

const previewPath = resolve("brand/migo-logo-preview.html");
const html = existsSync(previewPath) ? readFileSync(previewPath, "utf8") : "";
const concepts = ["a1", "a2", "b1", "b2", "c1", "c2"];
const checks = [
  ["preview document", /<!doctype html>/i],
  ["responsive viewport", /<meta name="viewport"/],
  ["selected preview live region", /id="selected-preview"[^>]+aria-live="polite"/],
  ["light theme control", /data-theme-choice="light"/],
  ["dark theme control", /data-theme-choice="dark"/],
  ["16 px specimens", /data-size="16"/],
  ["32 px specimens", /data-size="32"/],
  ["monochrome specimens", /data-treatment="mono"/],
  ["hash navigation", /location\.hash/],
  ["pressed state", /aria-pressed/],
  ["preview theme state", /previewTheme/],
];

for (const concept of concepts) {
  checks.push([`${concept} card`, new RegExp(`data-concept="${concept}"`)]);
  checks.push([`${concept} symbol`, new RegExp(`id="mark-${concept}"`)]);
  checks.push([`${concept} button`, new RegExp(`<button[^>]+data-select="${concept}"`)]);
}

const failures = checks
  .filter(([, pattern]) => !pattern.test(html))
  .map(([name]) => name);

if (/(?:src|href)="https?:\/\//.test(html) || /<link[^>]+rel="stylesheet"/.test(html)) {
  failures.push("external resource found");
}

if (failures.length) {
  console.error(`Logo preview contract failed:\n- ${failures.join("\n- ")}`);
  process.exit(1);
}

console.log(`Logo preview contract passed: ${checks.length} checks`);
