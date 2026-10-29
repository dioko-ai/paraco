// This host-only bridge is never bundled into an application worker.
import { pathToFileURL } from 'node:url';
import { realpathSync } from 'node:fs';

export async function complete(provider, sdk, { model, prompt }, cwd = process.cwd()) {
  if (provider === 'codex') {
    const codex = new sdk.Codex({ config: {
      features: { shell_tool: false, unified_exec: false, shell_snapshot: false,
        multi_agent: false, apps: false, remote_plugin: false, hooks: false,
        memories: false, goals: false, code_mode: { enabled: false } },
      tools: { view_image: false }, forced_login_method: 'chatgpt',
    }, configOverrides: ['mcp_servers={}', 'plugins={}', 'apps={_default={enabled=false}}'] });
    const thread = codex.startThread({ model, workingDirectory: cwd,
      skipGitRepoCheck: true, sandboxMode: 'read-only', approvalPolicy: 'never',
      networkAccessEnabled: false, webSearchMode: 'disabled' });
    return (await thread.run(prompt)).finalResponse;
  }
  if (provider === 'claude') {
    let result;
    for await (const message of sdk.query({ prompt, options: {
      model, cwd, tools: [], mcpServers: {}, settingSources: [],
      permissionMode: 'dontAsk', persistSession: false, maxTurns: 1,
      canUseTool: async () => ({ behavior: 'deny', message: 'Tools are unavailable for app completions.' }),
    } })) {
      if (message.type === 'result') {
        if (message.is_error || message.subtype !== 'success') throw new Error('SDK failed');
        result = message.result;
      }
    }
    if (typeof result !== 'string') throw new Error('SDK returned no completion');
    return result;
  }
  throw new Error('Unknown SDK');
}

function isMain() {
  if (!process.argv[1]) return false;
  try { return import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href; }
  catch { return false; }
}

if (isMain()) {
  try {
    let input = '';
    for await (const chunk of process.stdin) {
      input += chunk;
      if (input.length > 65536) throw new Error('Input too large');
    }
    const sdk = await import(pathToFileURL(process.argv[3]).href);
    const text = await complete(process.argv[2], sdk, JSON.parse(input));
    if (typeof text !== 'string') throw new Error('Invalid SDK output');
    process.stdout.write(JSON.stringify({ text }));
  } catch { process.exitCode = 1; }
}
