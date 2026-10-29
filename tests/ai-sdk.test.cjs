const { test } = require('node:test');
const assert = require('node:assert/strict');
const bridge = () => import('../runtime/ai-sdk.mjs');
test('Codex uses a new read-only thread, explicit model, and no approvals or network tools', async () => {
  let config, options, input;
  class Codex {
    constructor(value) { config = value; }
    startThread(value) { options = value; return { run: async prompt => { input = prompt; return { finalResponse: 'answer' }; } }; }
  }
  const { complete } = await bridge();
  assert.equal(await complete('codex', { Codex }, { model: 'chosen', prompt: 'private prompt' }, '/empty'), 'answer');
  assert.equal(config.config.features.shell_tool, false);
  assert.equal(options.sandboxMode, 'read-only');
  assert.equal(options.approvalPolicy, 'never');
  assert.equal(options.networkAccessEnabled, false);
  assert.equal(options.model, 'chosen');
  assert.equal(input, 'private prompt');
});
test('Claude disables tools and inherited settings and returns only successful final output', async () => {
  let options;
  const sdk = { query: async function* (value) { options = value.options; yield { type: 'assistant', content: 'intermediate' }; yield { type: 'result', subtype: 'success', result: 'answer' }; } };
  const { complete } = await bridge();
  assert.equal(await complete('claude', sdk, { model: 'chosen', prompt: 'prompt' }), 'answer');
  assert.deepEqual(options.tools, []);
  assert.deepEqual(options.settingSources, []);
  assert.equal((await options.canUseTool()).behavior, 'deny');
  await assert.rejects(complete('claude', { query: async function* () { yield { type: 'result', subtype: 'error_max_turns', is_error: true }; } }, { model: 'chosen', prompt: 'prompt' }));
});

test('SDK CLI runs through a symlink and imports without executing the CLI', async () => {
  const fs = require('node:fs');
  const os = require('node:os');
  const path = require('node:path');
  const { spawnSync } = require('node:child_process');
  const { pathToFileURL } = require('node:url');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'paraco-sdk-'));
  try {
    const script = path.join(dir, 'bridge.mjs');
    const alias = path.join(dir, 'alias.mjs');
    const sdk = path.join(dir, 'sdk.mjs');
    fs.copyFileSync(path.join(__dirname, '../runtime/ai-sdk.mjs'), script);
    fs.symlinkSync(script, alias);
    fs.writeFileSync(sdk, "export async function* query({prompt}) { yield {type:'result', subtype:'success', result:prompt}; }");
    const result = spawnSync(process.execPath, [alias, 'claude', sdk], {
      input: JSON.stringify({ model: 'test', prompt: 'hello' }), encoding: 'utf8', timeout: 5000,
    });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), { text: 'hello' });
    for (const argv of ['delete process.argv[1]', "process.argv[1]='/nonexistent/paraco-test.mjs'"]) {
      const imported = spawnSync(process.execPath, ['--input-type=module', '--eval',
        `${argv}; await import(${JSON.stringify(pathToFileURL(script).href)}); process.stdout.write('imported');`],
      { encoding: 'utf8', timeout: 5000 });
      assert.equal(imported.status, 0, imported.stderr);
      assert.equal(imported.stdout, 'imported');
    }
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});
