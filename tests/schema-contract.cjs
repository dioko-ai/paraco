const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const Ajv2020 = require("ajv/dist/2020");

const schema = JSON.parse(readFileSync("docs/paraco-manifest.schema.json", "utf8"));
const validate = new Ajv2020({ strict: true }).compile(schema);
for (const fixture of [
  { name: "vitals", entrypoint: "main.ts", capabilities: ["system"], targets: ["local"] },
  { name: "hello", entrypoint: "main.ts", capabilities: [] },
  { schemaVersion: 1, name: "ai-example", entrypoint: "main.ts", capabilities: ["ai"] },
  { name: "targets", entrypoint: "main.ts", capabilities: [], targets: ["local", "cloudflare"], outputs: { report: { mediaType: "application/json", schema: { type: "object" } } } },
]) assert.equal(validate(fixture), true, JSON.stringify(validate.errors));
for (const fixture of [
  { schemaVersion: 2, name: "hello", entrypoint: "main.ts", capabilities: [] },
  { name: "hello", entrypoint: "main.ts", capabilities: ["ai", "ai"] },
  { name: "hello-", entrypoint: "main.ts", capabilities: [] },
  { name: "Hello", entrypoint: "main.ts", capabilities: [] },
  { name: "hello", entrypoint: "main.ts", capabilities: [], unexpected: true },
  { name: "hello", entrypoint: "main.ts", capabilities: [], targets: [] },
  { name: "hello", entrypoint: "main.ts", capabilities: [], targets: ["local", "local"] },
  { name: "hello", entrypoint: "main.ts", capabilities: [], outputs: { "Bad Name": { mediaType: "application/json" } } },
  { name: "hello", entrypoint: "main.ts", capabilities: [], outputs: { report: { mediaType: " " } } },
]) assert.equal(validate(fixture), false, `fixture unexpectedly accepted: ${JSON.stringify(fixture)}`);
console.log("manifest schema fixtures passed");

const marketplaceSchema = JSON.parse(readFileSync("docs/marketplace.schema.json", "utf8"));
assert.deepEqual([...marketplaceSchema.properties.apps.items.properties.capabilities.items.enum].sort(), [...schema.properties.capabilities.items.enum].sort(), "marketplace and manifest must agree on capability names");
