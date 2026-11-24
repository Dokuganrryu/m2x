// WHY: runs the app's pure sections (firmware facts, EQ math, APO fit, demo protocol) under Node with no browser: node tools/m2x-studio/test.mjs
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
const start = html.indexOf('"use strict";'), end = html.indexOf("5. App state");
assert.ok(start > 0 && end > start, "index.html keeps its numbered script sections");
const body = html.slice(start, html.lastIndexOf("/* ===", end));
const api = new Function(body + "\nreturn { FW, BUILTIN, SAMPLE_APO, peakOf, zeroClipTrim, parseApo, fitBands, DemoDevice };")();

const failures = [];
const check = async (name, fn) => {
  try { await fn(); console.log("ok   " + name); } catch (e) { failures.push(name); console.log("FAIL " + name + "\n     " + e.message); }
};

for (const key of ["clarity", "warm"]) {
  for (const fs of [48000, 96000]) {
    await check(`${key} at ${fs} Hz: trim is the zero-clip rule of its own peak`, () => {
      const p = BUILTIN_PEAK(key, fs);
      assert.equal(api.zeroClipTrim(p), api.BUILTIN[key].trim);
    });
  }
}
function BUILTIN_PEAK(key, fs) { return api.peakOf(api.BUILTIN[key][fs], fs).db; }

await check("built-in trims match crates/device/src/preset/tables.rs", () => {
  assert.equal(api.BUILTIN.clarity.trim, -2.6);
  assert.equal(api.BUILTIN.warm.trim, -4.2);
  assert.equal(api.BUILTIN.flat.trim, 0);
  assert.equal(api.BUILTIN.clarity[96000].length, api.FW.bands);
  assert.equal(api.BUILTIN.warm[96000].length, api.FW.bands);
});

await check("sample APO parses to 16 bands with its preamp", () => {
  const apo = api.parseApo(api.SAMPLE_APO);
  assert.equal(apo.bands.length, 16);
  assert.equal(apo.preamp, -4);
  assert.equal(apo.skipped, 0);
});

await check("sample APO fits into 14 bands within 0.5 dB at 48 kHz", async () => {
  const fit = await api.fitBands(api.parseApo(api.SAMPLE_APO).bands, 48000);
  assert.equal(fit.bands.length, api.FW.bands);
  assert.ok(fit.max < 0.5, `max error ${fit.max.toFixed(3)} dB`);
});

await check("demo device answers the firmware's preset replies", () => {
  globalThis.performance ??= { now: () => 0 };
  const d = new api.DemoDevice();
  assert.deepEqual(d.reply("preset clarity"), ["preset clarity trim=-2.6 bands=14 path=eq"]);
  assert.deepEqual(d.reply("preset eq2"), ["preset warm trim=-4.2 bands=14 path=eq"]);
  assert.deepEqual(d.reply("preset raw"), ["preset flat trim=0.0 bands=0 path=bit-perfect"]);
  assert.deepEqual(d.reply("preset loud"), ["refused: bad arguments"]);
  assert.deepEqual(d.reply("x".repeat(65)), ["refused: line too long"]);
});

console.log(failures.length ? `${failures.length} failed` : "all passed");
process.exit(failures.length ? 1 : 0);
