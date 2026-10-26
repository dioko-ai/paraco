import { handleRequest } from "./main.ts";

Deno.test("serves the welcome page", async () => {
  const response = handleRequest(new Request("http://localhost/"));
  const html = await response.text();
  if (response.status !== 200) throw new Error(`unexpected status: ${response.status}`);
  if (!response.headers.get("content-type")?.startsWith("text/html")) {
    throw new Error("welcome page must be HTML");
  }
  if (!html.includes("Hello,") || !html.includes("Your first Paraco app")) {
    throw new Error("welcome page content is missing");
  }
});

Deno.test("health endpoint returns JSON", async () => {
  const response = handleRequest(new Request("http://localhost/health"));
  const body = await response.json();
  if (response.status !== 200 || body.status !== "ok" || body.name !== "hello-world") {
    throw new Error("health response is incorrect");
  }
});

Deno.test("unknown paths return 404", () => {
  const response = handleRequest(new Request("http://localhost/missing"));
  if (response.status !== 404) throw new Error(`unexpected status: ${response.status}`);
});
