const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const Ajv2020 = require("ajv/dist/2020");

const schema = JSON.parse(readFileSync("docs/paraco-manifest.schema.json", "utf8"));
const validate = new Ajv2020({ strict: true }).compile(schema);
for (const fixture of [
  { name: "hello", entrypoint: "main.ts", capabilities: [] },
  { schemaVersion: 1, name: "ai-example", entrypoint: "main.ts", capabilities: ["ai"] },
]) assert.equal(validate(fixture), true, JSON.stringify(validate.errors));
for (const fixture of [
  { schemaVersion: 2, name: "hello", entrypoint: "main.ts", capabilities: [] },
  { name: "hello", entrypoint: "main.ts", capabilities: ["ai", "ai"] },
  { name: "Hello", entrypoint: "main.ts", capabilities: [] },
  { name: "hello", entrypoint: "main.ts", capabilities: [], unexpected: true },
]) assert.equal(validate(fixture), false, `fixture unexpectedly accepted: ${JSON.stringify(fixture)}`);
console.log("manifest schema fixtures passed");
