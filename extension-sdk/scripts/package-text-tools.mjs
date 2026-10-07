import { spawnSync } from "node:child_process";
import { access, copyFile, mkdir, mkdtemp, readFile, rename, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const sdkRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = path.resolve(sdkRoot, "..");
const compiler = path.join(repositoryRoot, "launcher", "node_modules", "typescript", "bin", "tsc");
const manifestPath = path.join(sdkRoot, "examples", "text-tools.extension.json");
const targetRoot = path.join(repositoryRoot, "target", "extension-sdk-examples");

await access(compiler).catch(() => {
  throw new Error(
    "The locked TypeScript compiler is missing at launcher/node_modules; install the existing launcher toolchain before packaging.",
  );
});
await mkdir(targetRoot, { recursive: true });
const temporaryRoot = await mkdtemp(path.join(targetRoot, ".text-tools-build-"));
const stagingPath = path.join(temporaryRoot, "package");
const packagePath = path.join(
  targetRoot,
  `text-tools-${path.basename(temporaryRoot).slice(".text-tools-build-".length)}`,
);

try {
  const compilerOutput = path.join(temporaryRoot, "compiled");
  const compile = spawnSync(
    process.execPath,
    [
      compiler,
      "--project",
      path.join(sdkRoot, "tsconfig.json"),
      "--noEmit",
      "false",
      "--outDir",
      compilerOutput,
    ],
    { cwd: repositoryRoot, stdio: "inherit" },
  );
  if (compile.error) throw compile.error;
  if (compile.status !== 0) {
    throw new Error(`TypeScript compilation failed with status ${compile.status ?? "signal"}`);
  }

  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  if (manifest.entrypoints?.[0]?.file !== "dist/main.js") {
    throw new Error("Text Tools manifest must declare dist/main.js as its entrypoint.");
  }
  const entrypointPath = path.join(compilerOutput, "examples", "text-tools.js");
  const stagedEntrypoint = path.join(stagingPath, "dist", "main.js");
  await mkdir(path.dirname(stagedEntrypoint), { recursive: true });
  await copyFile(manifestPath, path.join(stagingPath, "manifest.json"));
  await copyFile(entrypointPath, stagedEntrypoint);
  await rename(stagingPath, packagePath);
  console.log(`Installable Text Tools package: ${packagePath}`);
} finally {
  await rm(temporaryRoot, { recursive: true, force: true });
}
