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
