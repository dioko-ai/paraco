// Fail early with actionable messages rather than allowing runtime/browser
// verification to be mistaken for a successful suite when prerequisites miss.
const { execFileSync } = require("node:child_process");
const { existsSync } = require("node:fs");

function version(command, args, expected) {
  let output;
  try {
    output = execFileSync(command, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  } catch {
    throw new Error(`${command} is required for this verification run`);
  }
  const firstLine = output.split(/\r?\n/, 1)[0];
  if (firstLine !== expected && !firstLine.startsWith(`${expected} `)) {
    throw new Error(`${command} must report ${expected}; got ${JSON.stringify(output.trim())}`);
  }
}

try {
  const workerd = process.env.WORKERD_BIN || "workerd";
  let runtimeVersion;
  try {
    runtimeVersion = execFileSync(workerd, ["--version"], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).split(/\r?\n/, 1)[0];
  } catch {
    throw new Error(`${workerd} is required for this verification run`);
  }
  if (runtimeVersion !== "workerd 2026-10-26") {
    throw new Error(`${workerd} must report workerd 2026-10-26; got ${JSON.stringify(runtimeVersion)}`);
  }
  version("esbuild", ["--version"], "0.28.2");
  if (process.env.CI) {
    version(process.execPath, ["--version"], "v22.14.0");
  } else if (Number(process.versions.node.split(".")[0]) < 22) {
    throw new Error("Node.js 22 or newer is required for local verification");
  }
  // Playwright's downloaded browser is deliberately checked before a test
  // starts so a missing cache cannot look like an application failure.
  if (!existsSync("node_modules/@playwright/test")) {
    throw new Error("npm ci is required before browser verification");
  }
  const { chromium } = require("@playwright/test");
  if (!existsSync(chromium.executablePath())) {
    throw new Error("Playwright Chromium is required; run `npx playwright install chromium`");
  }
} catch (error) {
  console.error(`verification prerequisite failed: ${error.message}`);
  process.exitCode = 1;
}
