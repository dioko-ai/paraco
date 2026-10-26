const { test } = require('node:test');
const assert = require('node:assert/strict');
const esbuild = require('esbuild');
const { chromium } = require('@playwright/test');

const bundle = esbuild.buildSync({
  entryPoints: [new URL('../runtime/artifact-picker.ts', `file://${__filename}`).pathname],
  bundle: true,
  platform: 'browser',
  format: 'iife',
  globalName: 'ArtifactPicker',
  write: false,
}).outputFiles[0].text;

test('artifact picker previews bounded queries and explicit pins, handles errors and stale responses, and cleans up', { timeout: 30000 }, async t => {
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage();
    await page.setContent('<main id="mount"></main>');
    await page.addScriptTag({ content: bundle });
    await page.evaluate(() => {
      window.selected = [];
      window.calls = [];
      const first = { id: 'art-1', output: 'report', mediaType: 'text/plain', project: 'alpha', status: 'ready', tags: ['daily'], createdAt: 123 };
      const pinned = { id: 'art-pinned', output: 'report', mediaType: 'text/plain', project: 'archive', status: 'done', createdAt: 456 };
      window.api = {
        list: async query => {
          window.calls.push({ action: 'list', query });
          if (query.status === 'fail') throw new Error('preview failed');
          if (query.status === 'slow') return new Promise(resolve => { window.resolveSlow = () => resolve([first]); });
          return [first];
        },
        get: async id => {
          window.calls.push({ action: 'get', id });
          return pinned;
        },
      };
      window.picker = ArtifactPicker.mountArtifactPicker(document.querySelector('#mount'), window.api, ref => window.selected.push(ref));
    });

    const field = name => page.locator(`input[name="${name}"]`);
    const preview = page.getByRole('button', { name: 'Preview query' });
    const save = page.getByRole('button', { name: 'Save selected reference' });
    await field('applicationId').fill('app-7');
    await field('output').fill('report');
    await field('project').fill('alpha');
    await field('tag').fill('daily');
    await field('status').fill('ready');
    await page.getByLabel('Dynamic reference window').selectOption('5');
    await preview.click();
    const result = page.getByRole('button', { name: /report · art-1/ });
    await result.waitFor();
    const friendlyDate = await page.evaluate(() => new Date(123).toLocaleString());
    assert.match(await result.textContent(), new RegExp(`text/plain · alpha · ready · daily · ${friendlyDate.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`));
    assert.equal(await save.isEnabled(), false);
    await result.click();
    assert.equal(await save.isEnabled(), true);
    await save.click();
    assert.deepEqual(await page.evaluate(() => window.selected[0]), {
      kind: 'dynamic',
      query: { limit: 5, applicationId: 'app-7', output: 'report', project: 'alpha', tag: 'daily', status: 'ready' },
    });
    assert.deepEqual(await page.evaluate(() => window.calls[0]), {
      action: 'list',
      query: { limit: 5, applicationId: 'app-7', output: 'report', project: 'alpha', tag: 'daily', status: 'ready' },
    });

    await field('artifactId').fill('art-pinned');
    await page.getByRole('button', { name: 'Preview pinned artifact' }).click();
    const pin = page.getByRole('button', { name: /report · art-pinned/ });
    await pin.waitFor();
    await pin.evaluate(button => { window.staleButton = button; });
    await pin.click();
    await save.click();
    assert.deepEqual(await page.evaluate(() => window.selected[1]), { kind: 'permanent', ids: ['art-pinned'] });
    assert.deepEqual(await page.evaluate(() => window.calls[1]), { action: 'get', id: 'art-pinned' });

    await field('status').fill('fail');
    assert.equal(await page.locator('ul button').count(), 0, 'editing a filter removes prior results');
    assert.equal(await save.isEnabled(), false, 'editing a filter invalidates the saved choice');
    await page.evaluate(() => window.staleButton.click());
    assert.equal(await save.isEnabled(), false, 'a detached old result cannot restore the choice');
    await preview.click();
    await page.getByRole('status').getByText('preview failed').waitFor();
    assert.equal(await save.isEnabled(), false);
    assert.equal(await page.locator('ul button').count(), 0, 'failed preview leaves no stale results');

    await field('status').fill('slow');
    await page.evaluate(() => window.staleButton.click());
    assert.equal(await save.isEnabled(), false, 'editing during a failed preview cannot restore the old choice');
    await preview.click();
    await page.getByRole('status').getByText('Loading artifacts…').waitFor();
    assert.equal(await page.locator('ul button').count(), 0, 'pending preview clears the old result buttons');
    await page.evaluate(() => window.staleButton.click());
    assert.equal(await save.isEnabled(), false, 'an old result cannot be selected while a preview is pending');
    await field('status').fill('ready');
    await page.evaluate(() => window.staleButton.click());
    assert.equal(await save.isEnabled(), false, 'editing a pending query keeps old selections invalid');
    await preview.click();
    await page.getByRole('button', { name: /report · art-1/ }).waitFor();
    await page.evaluate(() => window.resolveSlow());
    await page.waitForTimeout(25);
    assert.equal(await save.isEnabled(), false, 'older response must not enable selection');
    assert.match(await page.getByRole('status').textContent(), /dynamic query \(latest 5\)/);

    await page.evaluate(() => window.picker.destroy());
    assert.equal(await page.locator('#mount').locator('section').count(), 0);
  } finally {
    await browser.close();
  }
});
