// Real Chromium smoke test for the authenticated management dashboard.
// It is intentionally independent of the DOM-unit tests in management.test.cjs.
const assert = require("node:assert/strict");
const { mkdir, mkdtemp, writeFile } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { spawn } = require("node:child_process");
const net = require("node:net");
const { chromium } = require("@playwright/test");

const root = process.cwd();
const bin = process.env.PARACO_BIN || join(root, "target", "debug", "paraco");
const timeout = 15_000;

async function waitFor(check, description) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const result = await check();
    if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  throw new Error(`timed out waiting for ${description}`);
}

async function freePort() {
  const listener = net.createServer();
  await new Promise(resolve => listener.listen(0, "127.0.0.1", resolve));
  const port = listener.address().port;
  await new Promise((resolve, reject) => listener.close(error => error ? reject(error) : resolve()));
  return port;
}

(async () => {
  const dir = await mkdtemp(join(tmpdir(), "paraco-browser-"));
  const isolatedTmp = join(dir, "tmp");
  await mkdir(isolatedTmp);
  const port = await freePort();
  const app = join(dir, "hello");
  const other = join(dir, "other");
  await require("node:fs/promises").mkdir(app);
  await require("node:fs/promises").mkdir(other);
  await writeFile(join(app, "paraco.json"), JSON.stringify({ name: "hello", entrypoint: "main.ts", capabilities: [] }));
  await writeFile(join(app, "main.ts"), 'console.log("browser smoke"); export default { fetch() { return new Response("<h1>hello</h1>", {headers:{"content-type":"text/html"}}); } };');
  await writeFile(join(other, "paraco.json"), JSON.stringify({ name: "other", entrypoint: "main.ts", capabilities: [] }));
  await writeFile(join(other, "main.ts"), 'export default { fetch() { return new Response("<h1>other</h1>", {headers:{"content-type":"text/html"}}); } };');
  await writeFile(join(dir, "server.json"), JSON.stringify({ apps: [{ path: "hello" }, { path: "other" }] }));
  const child = spawn(bin, ["--log-dir", join(dir, "logs"), "serve", "--config", join(dir, "server.json"), "--port", String(port)], {
    cwd: root, detached: process.platform !== "win32", stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, TMPDIR: isolatedTmp }
  });
  let output = "";
  child.stdout.on("data", data => { output += data; });
  child.stderr.on("data", data => { output += data; });
  const exited = () => new Promise(resolve => child.once("exit", resolve));
  const stop = async () => {
    if (child.exitCode !== null) return;
    child.kill("SIGINT");
    const graceful = await Promise.race([exited().then(() => true), new Promise(resolve => setTimeout(() => resolve(false), timeout))]);
    if (graceful) return;
    try {
      if (process.platform === "win32") child.kill("SIGKILL");
      else process.kill(-child.pid, "SIGKILL");
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
    await Promise.race([exited(), new Promise(resolve => setTimeout(resolve, 2000))]);
    throw new Error("server shutdown timed out and was force-terminated");
  };
  try {
    const url = await waitFor(() => output.match(/management dashboard (http:\/\/[^\s]+)/)?.[1], "management URL");
    const browser = await chromium.launch({ headless: true });
    try {
      const page = await browser.newPage();
      await page.goto(url);
      await page.getByText("2 of 2 apps running").waitFor({ timeout });
      const origins = await page.locator('a[href^="http://app-"]').evaluateAll(links => links.map(link => link.href));
      assert.equal(origins.length, 2);
      assert.notEqual(new URL(origins[0]).origin, new URL(origins[1]).origin);
      const [first] = await Promise.all([
        page.context().waitForEvent('page'),
        page.locator('a[href^="http://app-"]').first().click()
      ]);
      await first.waitForLoadState();
      assert.equal(new URL(first.url()).origin, new URL(origins[0]).origin);
      await first.getByRole('heading', { name: 'hello' }).waitFor({ timeout });
      await first.evaluate(() => { localStorage.setItem("isolation", "one"); document.cookie = "hostonly=one"; });
      const [second] = await Promise.all([
        page.context().waitForEvent('page'),
        page.locator('a[href^="http://app-"]').nth(1).click()
      ]);
      await second.waitForLoadState();
      assert.equal(new URL(second.url()).origin, new URL(origins[1]).origin);
      await second.getByRole('heading', { name: 'other' }).waitFor({ timeout });
      assert.equal(await second.evaluate(() => localStorage.getItem("isolation")), null);
      assert.doesNotMatch(await second.evaluate(() => document.cookie), /hostonly=one/);
      const gateway = `http://127.0.0.1:${new URL(origins[0]).port}/`;
      await page.goto(gateway);
      await page.locator('a[href^="http://app-"]').first().click();
      await page.getByRole('heading', { name: 'hello' }).waitFor({ timeout });
      await page.goto(url);
      await page.getByRole("button", { name: "logs hello" }).click();
      await page.getByText("Logs: hello").waitFor({ timeout });
      await page.getByRole("button", { name: "restart hello" }).click();
      await page.getByText("restart requested for hello", { exact: false }).waitFor({ timeout });
      await page.getByText("2 of 2 apps running").waitFor({ timeout });
      assert.match(await page.locator("#log-output").textContent(), /browser smoke|No retained logs/);
    } finally { await browser.close(); }
  } finally { await stop(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
