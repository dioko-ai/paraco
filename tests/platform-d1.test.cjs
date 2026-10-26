const assert = require('node:assert/strict');
const { DatabaseSync } = require('node:sqlite');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createRequire } = require('node:module');
const { build } = require('esbuild');
const { test } = require('node:test');

const root = path.resolve(__dirname, '..');
const temp = mkdtempSync(path.join(os.tmpdir(), 'paraco-platform-d1-'));
const compiled = path.join(temp, 'd1-service.cjs');
const createLocalRequire = createRequire(path.join(root, 'package.json'));
// Load only after esbuild has written the CommonJS bundle.
let service;
const ready = build({
  entryPoints: [path.join(root, 'runtime/d1-service.ts')],
  outfile: compiled,
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node22',
}).then(() => {
  service = createLocalRequire(compiled);
});

class D1Statement {
  constructor(db, sql, values = []) { this.db = db; this.sql = sql; this.values = values; }
  bind(...values) { this.values = values; return this; }
  async first() { return this.db.prepare(this.sql).get(...this.values) ?? null; }
  async all() { return { results: this.db.prepare(this.sql).all(...this.values) }; }
  async run() { return this.db.prepare(this.sql).run(...this.values); }
}

class D1Database {
  constructor(filename = ':memory:') {
    this.db = new DatabaseSync(filename);
    this.db.exec(readFileSync(path.join(root, 'runtime/platform-schema.sql'), 'utf8'));
  }
  prepare(sql) { return new D1Statement(this.db, sql); }
  async batch(statements) {
    this.db.exec('BEGIN IMMEDIATE');
    try {
      const result = [];
      for (const statement of statements) result.push(await statement.all());
      this.db.exec('COMMIT');
      return result;
    } catch (error) {
      this.db.exec('ROLLBACK');
      throw error;
    }
  }
  close() { this.db.close(); }
}

const principal = (applicationId = 'app', installationId = 'install', datasets = {}, outputs = {}) => ({
  applicationId, installationId, datasets, outputs,
});
const shared = (id = 'team', write = true) => ({ data: { id, write } });
const call = (db, who, request) => service.execute(db, who, request);
const record = (dataset, action, key, rest = {}) => ({ dataset, action, key, ...rest });
const draft = (content, idempotencyKey, extra = {}) => ({
  output: 'report', mediaType: 'application/json', content, idempotencyKey, ...extra,
});
const publish = (db, who, value) => call(db, who, { dataset: 'data', action: 'artifact.publish', draft: value });

test('D1 adapter loads and service fixture compiles', async () => {
  await ready;
  assert.equal(typeof service.execute, 'function');
  assert.equal(typeof service.default.fetch, 'function');
});

test('record compare-and-swap permits one concurrent writer and preserves tombstone version', async () => {
  await ready;
  const db = new D1Database();
  const who = principal();
  try {
    const attempts = await Promise.allSettled([
      call(db, who, record('private', 'record.put', 'key', { expectedVersion: null, value: 'one' })),
      call(db, who, record('private', 'record.put', 'key', { expectedVersion: null, value: 'two' })),
    ]);
    assert.equal(attempts.filter((x) => x.status === 'fulfilled').length, 1);
    assert.match(attempts.find((x) => x.status === 'rejected').reason.message, /conflict/);
    const deleted = await call(db, who, record('private', 'record.delete', 'key', { expectedVersion: 1 }));
    assert.equal(deleted.version, 2);
    assert.equal(deleted.deleted, true);
    await assert.rejects(call(db, who, record('private', 'record.put', 'key', { expectedVersion: 1, value: 'ABA' })), /conflict/);
    const restored = await call(db, who, record('private', 'record.put', 'key', { expectedVersion: 2, value: 'restored' }));
    assert.equal(restored.version, 3);
  } finally { db.close(); }
});

test('private scope is installation-local; shared grants cross installations and honor read-only', async () => {
  await ready;
  const db = new D1Database();
  try {
    await call(db, principal('app-a', 'one'), record('private', 'record.put', 'secret', { expectedVersion: null, value: 9 }));
    assert.equal(await call(db, principal('app-b', 'two'), record('private', 'record.get', 'secret')), null);
    const writer = principal('writer', 'one', shared('common', true));
    const reader = principal('reader', 'two', shared('common', false));
    await call(db, writer, record('data', 'record.put', 'shared-key', { expectedVersion: null, value: 10 }));
    assert.equal((await call(db, reader, record('data', 'record.get', 'shared-key'))).value, 10);
    await assert.rejects(call(db, reader, record('data', 'record.put', 'shared-key', { expectedVersion: 1, value: 11 })), /read-only/);
    await assert.rejects(call(db, principal(), record('missing', 'record.get', 'x')), /not granted/);
  } finally { db.close(); }
});

test('artifact idempotency returns the original and rejects changed content and media type', async () => {
  await ready;
  const db = new D1Database();
  const who = principal('app', 'install', shared(), { report: { mediaType: 'application/json' } });
  try {
    const first = await publish(db, who, draft({ n: 1 }, 'same'));
    const again = await publish(db, who, draft({ n: 1 }, 'same'));
    assert.equal(again.id, first.id);
    await assert.rejects(publish(db, who, draft({ n: 2 }, 'same')), /idempotency key reused/);
    await assert.rejects(publish(db, who, { output: 'report', mediaType: 'text/plain', content: 'x' }), /declared output/);
    const found = await call(db, who, { dataset: 'data', action: 'artifact.get', id: first.id });
    assert.deepEqual(found.content, { n: 1 });
  } finally { db.close(); }
});

test('dynamic resolution snapshots its initial selection and slot reuse conflicts', async () => {
  await ready;
  const db = new D1Database();
  const who = principal('app', 'install', shared());
  const request = (reference) => ({ dataset: 'data', action: 'artifact.resolve', executionId: 'run', slot: 'input', reference });
  try {
    const first = await publish(db, who, draft('old', 'old'));
    const reference = { kind: 'dynamic', query: { output: 'report' } };
    const resolved = await call(db, who, request(reference));
    assert.deepEqual(resolved.map((x) => x.id), [first.id]);
    await publish(db, who, draft('new', 'new'));
    assert.deepEqual(await call(db, who, request(reference)), resolved);
    await assert.rejects(call(db, who, request({ kind: 'dynamic', query: { tag: 'new' } })), /different reference/);
  } finally { db.close(); }
});

test('lineage resolves inputs only within the principal dataset', async () => {
  await ready;
  const db = new D1Database();
  const who = principal('producer', 'one', shared('team'));
  try {
    const source = await publish(db, who, draft('source', 'source'));
    const child = await publish(db, who, draft('child', 'child', { inputs: [source.id] }));
    const lineage = await call(db, principal('reader', 'two', shared('team', false)), { dataset: 'data', action: 'artifact.lineage', id: child.id });
    assert.deepEqual(lineage.inputs.map((x) => x.id), [source.id]);
    await assert.rejects(call(db, principal('outsider', 'three', shared('other', false)), { dataset: 'data', action: 'artifact.get', id: child.id }), /not found or not authorized/);
    await assert.rejects(call(db, principal('private', 'private-install'), {
      dataset: 'private', action: 'artifact.publish', draft: draft('bad', 'bad', { inputs: [source.id] }),
    }), /not found or not authorized/);
  } finally { db.close(); }
});

test('HTTP service trusts token principal and ignores forged principal fields in body', async () => {
  await ready;
  const db = new D1Database();
  const actual = principal('trusted-app', 'trusted-install', shared('trusted-dataset'));
  const forged = principal('forged-app', 'forged-install', shared('forged-dataset'));
  const env = { DB: db, PRINCIPALS: JSON.stringify({ good: actual }) };
  const send = (body, token = 'good') => service.default.fetch(new Request('https://service.test/', {
    method: 'POST', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: JSON.stringify(body),
  }), env);
  try {
    const response = await send({ operation: 'platform', request: { dataset: 'data', action: 'record.put', key: 'v', expectedVersion: null, value: 4 }, principal: forged });
    assert.equal(response.status, 200);
    const body = await response.json();
    assert.equal(body.value.version, 1);
    assert.equal(await call(db, principal('x', 'forged-install', shared('forged-dataset')), record('data', 'record.get', 'v')), null);
    assert.equal((await call(db, actual, record('data', 'record.get', 'v'))).value, 4);
    assert.equal((await send({ dataset: 'private', action: 'record.get', key: 'x' }, 'wrong')).status, 403);
  } finally { db.close(); }
});

test('expected scope must match the authenticated application, installation, and dataset', async () => {
  await ready;
  const db = new D1Database();
  const who = principal('app-1', 'install-1', shared('team'));
  const goodScope = { installationId: 'install-1', applicationId: 'app-1', datasetId: 'shared:team' };
  try {
    const accepted = await call(db, who, {
      dataset: 'data', action: 'record.put', key: 'scoped', expectedVersion: null, value: 1,
      expectedScope: goodScope,
    });
    assert.equal(accepted.version, 1);
    for (const expectedScope of [
      { ...goodScope, installationId: 'other-install' },
      { ...goodScope, applicationId: 'other-app' },
      { ...goodScope, datasetId: 'shared:other-team' },
    ]) {
      await assert.rejects(call(db, who, {
        dataset: 'data', action: 'record.get', key: 'scoped', expectedScope,
      }), /scope does not match/);
    }
    assert.equal((await call(db, who, { dataset: 'data', action: 'record.get', key: 'scoped' })).value, 1);
  } finally { db.close(); }
});

test('dataset grants reject non-boolean write permissions', async () => {
  await ready;
  const db = new D1Database();
  const malformed = principal('app', 'install', { data: { id: 'team', write: 'false' } });
  try {
    await assert.rejects(call(db, malformed, {
      dataset: 'data', action: 'record.put', key: 'must-not-write', expectedVersion: null, value: 1,
    }), /boolean write permission/);
    assert.equal(db.db.prepare('SELECT count(*) AS count FROM platform_records').get().count, 0);
  } finally { db.close(); }
});

test('legacy config and data storage stays scoped to the installation', async () => {
  await ready;
  const db = new D1Database();
  const env = { DB: db, PRINCIPALS: JSON.stringify({ token: principal('app', 'one') }) };
  const send = (body) => service.default.fetch(new Request('https://service.test/', {
    method: 'POST', headers: { authorization: 'Bearer token', 'content-type': 'application/json' }, body: JSON.stringify(body),
  }), env);
  try {
    for (const namespace of ['config', 'data']) {
      const saved = await send({ operation: 'set', namespace, key: 'key', value: namespace });
      assert.equal(saved.status, 200);
      assert.equal((await (await send({ operation: 'get', namespace, key: 'key' })).json()).value, namespace);
      env.PRINCIPALS = JSON.stringify({ token: principal('app', 'two') });
      assert.equal((await (await send({ operation: 'get', namespace, key: 'key' })).json()).value, null);
      env.PRINCIPALS = JSON.stringify({ token: principal('app', 'one') });
    }
  } finally { db.close(); }
});

test('request limits, unknown filters, and oversized tag arrays are rejected', async () => {
  await ready;
  const db = new D1Database();
  const who = principal('app', 'install', shared());
  try {
    await assert.rejects(call(db, who, { dataset: 'data', action: 'artifact.list', query: { unknown: 'x' } }), /unknown artifact filter/);
    await assert.rejects(call(db, who, { dataset: 'data', action: 'artifact.list', query: { limit: 101 } }), /limit must be/);
    await assert.rejects(publish(db, who, draft('x', 'large-tags', { tags: Array.from({ length: 101 }, (_, i) => `t${i}`) })), /at most 100/);
    await assert.rejects(call(db, who, { dataset: 'data', action: 'record.put', key: 'big', expectedVersion: null, value: 'x'.repeat(66000) }), /64 KiB/);
  } finally { db.close(); }
});

test('dataset quota abort rolls back a record write without changing existing data', async () => {
  await ready;
  const db = new D1Database();
  const who = principal('app', 'install', shared('quota'));
  try {
    const existing = JSON.stringify('x'.repeat(8 * 1024 * 1024 - 200));
    db.db.prepare('INSERT INTO platform_records(dataset,key,value,version,deleted) VALUES(?,?,?,1,0)')
      .run('shared:quota', 'seed', existing);
    await assert.rejects(
      call(db, who, record('data', 'record.put', 'new-key', { expectedVersion: null, value: 1 })),
      /quota exceeded/,
    );
    assert.equal(await call(db, who, record('data', 'record.get', 'new-key')), null);
    const saved = db.db.prepare('SELECT value,version FROM platform_records WHERE dataset=? AND key=?')
      .get('shared:quota', 'seed');
    assert.equal(saved.value, existing);
    assert.equal(saved.version, 1);
  } finally { db.close(); }
});

test.after(() => rmSync(temp, { recursive: true, force: true }));
