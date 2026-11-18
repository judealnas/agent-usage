import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const run = (args) => execFileSync("cargo", args, { cwd: root, encoding: "utf8" });
const target = process.env.TAURI_ENV_TARGET_TRIPLE || execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (.+)$/m)[1].trim();
const metadata = JSON.parse(run(["metadata", "--no-deps", "--format-version", "1", "--locked"]));
execFileSync("cargo", ["build", "-p", "agent-usage-cli", "--release", "--locked", "--target", target], { cwd: root, stdio: "inherit" });
const suffix = target.includes("windows") ? ".exe" : "";
const destination = fileURLToPath(new URL("../src-tauri/binaries/", import.meta.url));
mkdirSync(destination, { recursive: true });
copyFileSync(join(metadata.target_directory, target, "release", `agent-usage-cli${suffix}`), join(destination, `agent-usage-cli-${target}${suffix}`));
