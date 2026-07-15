import {cp, mkdir, readFile, writeFile} from "node:fs/promises";
import {dirname, join, resolve} from "node:path";
import {fileURLToPath} from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const [browser, outputArg, rootArg] = process.argv.slice(2);
if (!new Set(["chrome", "firefox"]).has(browser) || !outputArg) {
  console.error("usage: node extension/scripts/build.mjs <chrome|firefox> <output-directory> [project-root]");
  process.exit(2);
}

const projectRoot = resolve(rootArg ?? join(here, "../.."));
const extensionRoot = join(projectRoot, "extension");
const output = resolve(outputArg);
const cargo = await readFile(join(projectRoot, "Cargo.toml"), "utf8");
const packageSection = cargo.match(/\[package\]([\s\S]*?)(?:\n\[|$)/)?.[1] ?? "";
const version = packageSection.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (!version) throw new Error("Cargo package version not found");

const manifest = JSON.parse(await readFile(join(extensionRoot, "manifests", `${browser}.json`), "utf8"));
manifest.version = version;
await mkdir(output, {recursive: true});
for (const file of ["background.js", "queue.js", "content-core.js", "content.js"]) {
  await cp(join(extensionRoot, file), join(output, file));
}
await writeFile(join(output, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
