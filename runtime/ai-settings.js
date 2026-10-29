'use strict';
(() => {
  const field = name => document.querySelector(`#ai-${name}`);
  let settings;
  let busy = false;
  function visibility() {
    const custom = field('mode').value === 'custom';
    const cli = field('provider').value === 'codex';
    const supportsSdk = ['openai', 'anthropic'].includes(field('provider').value);
    if (!supportsSdk) field('transport').value = 'api';
    field('transport').querySelector('option[value="sdk"]').disabled = !supportsSdk;
    const sdk = field('transport').value === 'sdk';
    field('profile').hidden = !custom;
    field('api-fields').hidden = sdk || cli;
    field('advanced').hidden = !supportsSdk;
    field('codex-info').hidden = !cli;
    field('model').placeholder = cli ? 'Automatic (Codex default)' : 'Model ID from your provider';
    field('model').required = custom && !cli;
    field('sdk-fields').hidden = !sdk;
    field('mode').querySelector('option[value="inherit"]').disabled = !field('scope').value;
  }
  function populate() {
    const app = settings.apps.find(a => a.name === field('scope').value);
    const selected = app ? app.override : settings.default ? { mode: 'custom', profile: settings.default } : { mode: 'disabled' };
    field('mode').value = selected?.mode || 'inherit';
    const profile = selected?.profile;
    const cli = profile?.transport?.kind === 'codex_cli';
    field('provider').value = cli ? 'codex' : profile?.provider || 'codex';
    field('transport').value = profile?.transport?.kind?.endsWith('_sdk') ? 'sdk' : 'api';
    for (const name of ['model', 'endpoint']) field(name).value = profile?.[name] || '';
    if (cli && profile.model === 'default') field('model').value = '';
    field('advanced').open = field('transport').value === 'sdk';
    for (const name of ['node', 'module']) field(name).value = profile?.transport?.[name] || '';
    field('key').value = '';
    field('key').placeholder = profile?.credential_file ? 'Key saved. Leave blank to keep it.' : 'Enter an API key';
    visibility();
  }
  async function api(body) {
    const response = await fetch('/api/ai', {
      method: body ? 'POST' : 'GET', credentials: 'omit',
      headers: body ? { 'Content-Type': 'application/json' } : {},
      body: body ? JSON.stringify(body) : undefined, signal: AbortSignal.timeout(5000),
    });
    if (!response.ok) throw new Error(await response.text());
    return response.json();
  }
  field('refresh').addEventListener('click', async () => {
    if (busy) return;
    busy = true;
    try {
      settings = await api();
      const global = document.createElement('option');
      global.value = ''; global.textContent = 'All apps (global default)';
      field('scope').replaceChildren(global);
      for (const app of settings.apps) {
        const option = document.createElement('option');
        option.value = app.name;
        option.disabled = !app.requests_ai || app.explicit_ai;
        option.textContent = app.name + (app.explicit_ai ? ' — managed by configuration file' : !app.requests_ai ? ' — does not request AI' : '');
        field('scope').append(option);
      }
      field('form').hidden = false;
      field('scope').value = '';
      populate();
      field('status').textContent = '';
    } catch (error) { field('status').textContent = error.message; }
    finally { busy = false; }
  });
  field('scope').addEventListener('change', populate);
  field('provider').addEventListener('change', () => { field('model').value = ''; field('endpoint').value = ''; field('key').value = ''; });
  for (const name of ['mode', 'provider', 'transport']) field(name).addEventListener('change', visibility);
  field('form').addEventListener('submit', async event => {
    event.preventDefault();
    if (busy) return;
    busy = true;
    const button = field('form').querySelector('button[type="submit"]');
    button.disabled = true;
    try {
      const cli = field('provider').value === 'codex';
      const provider = cli ? 'openai' : field('provider').value;
      const sdk = field('transport').value === 'sdk';
      const profile = {
        provider, model: field('model').value.trim(),
        transport: cli ? { kind: 'codex_cli' } : sdk ? { kind: provider === 'openai' ? 'codex_sdk' : 'claude_sdk', node: field('node').value.trim(), module: field('module').value.trim() }
          : { kind: provider === 'anthropic' ? 'anthropic' : 'openai' },
        endpoint: cli || sdk ? null : field('endpoint').value.trim() || null, credential_file: null,
      };
      settings = await api({ app: field('scope').value || null, mode: field('mode').value,
        profile: field('mode').value === 'custom' ? profile : null, api_key: field('key').value || null });
      populate();
      field('status').textContent = 'Saved. New AI requests will use these settings.';
    } catch (error) { field('status').textContent = error.message; }
    finally { field('key').value = ''; busy = false; button.disabled = false; }
  });
})();
