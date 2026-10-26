import assert from "node:assert/strict";
import test from "node:test";
import app, { handleRequest } from "./main.ts";

test("serves the welcome page", async () => {
  const response = await app.fetch(new Request("http://localhost/"), {} as never);
  const html = await response.text();
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /^text\/html/);
  assert.match(html, /Hello,/);
  assert.match(html, /Your first Paraco app/);
});

test("health endpoint returns JSON", async () => {
  const response = handleRequest(new Request("http://localhost/health"));
  assert.deepEqual(await response.json(), { status: "ok", name: "hello-world" });
});

test("unknown paths return 404", () => {
  const response = handleRequest(new Request("http://localhost/missing"));
  assert.equal(response.status, 404);
});
