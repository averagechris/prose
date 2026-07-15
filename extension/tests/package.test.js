const assert = require("node:assert/strict");
const {mkdtempSync, readFileSync, rmSync} = require("node:fs");
const {tmpdir} = require("node:os");
const {join, resolve} = require("node:path");
const {spawnSync} = require("node:child_process");
const {test} = require("node:test");

const project = resolve(__dirname, "../..");
const cargoVersion = readFileSync(join(project, "Cargo.toml"), "utf8")
  .match(/\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)[1];

for (const browser of ["chrome", "firefox"]) {
  test(`generated ${browser} manifest is browser-specific and version-aligned`, () => {
    const parent = mkdtempSync(join(tmpdir(), "prose-extension-"));
    const output = join(parent, browser);
    const built = spawnSync(process.execPath, [join(project, "extension/scripts/build.mjs"), browser, output, project], {encoding: "utf8"});
    assert.equal(built.status, 0, built.stderr);
    const manifest = JSON.parse(readFileSync(join(output, "manifest.json"), "utf8"));
    assert.equal(manifest.version, cargoVersion);
    if (browser === "chrome") {
      assert.equal(manifest.background.service_worker, "background.js");
      assert.equal(manifest.background.scripts, undefined);
      assert.equal(manifest.browser_specific_settings, undefined);
    } else {
      assert.deepEqual(manifest.background.scripts, ["queue.js", "content-core.js", "background.js"]);
      assert.equal(manifest.background.service_worker, undefined);
      assert.equal(manifest.browser_specific_settings.gecko.id, "prose@averagechris");
    }
    assert.deepEqual(manifest.content_scripts[0].js, ["content-core.js", "content.js"]);
    rmSync(parent, {recursive: true, force: true});
  });
}
