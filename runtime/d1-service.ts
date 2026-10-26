/** Trusted capability service. Deploy separately from downloaded application code.
 * Apply platform-schema.sql first. Never give an app the DB or PRINCIPALS bindings.
 * D1 writes below use single conditional statements; resolution uses an atomic batch.
 */
interface Statement {
  bind(...values: unknown[]): Statement;
  first<T = Record<string, unknown>>(): Promise<T | null>;
  all<T = Record<string, unknown>>(): Promise<{results: T[]}>;
  run(): Promise<unknown>;
}
export interface Database {
  prepare(sql: string): Statement;
  batch<T = Record<string, unknown>>(statements: Statement[]): Promise<Array<{results: T[]}>>;
}
export interface Principal {
  applicationId: string;
  installationId: string;
  outputs?: Record<string, {mediaType: string; schema?: unknown}>;
  datasets?: Record<string, {id: string; write: boolean}>;
}
interface Environment { DB: Database; PRINCIPALS: string }
const encoder = new TextEncoder();
function identifier(value: unknown): string {
  if (typeof value !== 'string' || !value.length || encoder.encode(value).length > 128 || /[\u0000-\u001f\u007f-\u009f]/.test(value)) throw new Error('identifier must contain 1–128 UTF-8 bytes without control characters');
  return value;
}
function object(value: unknown): Record<string, any> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('object required');
  return value;
}
/** Stable recursively sorted representation matches serde_json's map encoding. */
function canonical(value: any): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value && typeof value === 'object') return `{${Object.keys(value).sort().map(k => `${JSON.stringify(k)}:${canonical(value[k])}`).join(',')}}`;
  return JSON.stringify(value);
}
function discovery(dataset: string, input: unknown): {sql: string; values: unknown[]} {
  const query = object(input);
  if (Object.keys(query).some(k => !['applicationId','installationId','output','project','tag','status','limit'].includes(k))) throw new Error('unknown artifact filter');
  const limit = query.limit === undefined ? 20 : query.limit;
  if (!Number.isInteger(limit) || limit < 1 || limit > 100) throw new Error('limit must be 1–100');
  const values: unknown[] = [dataset];
  let sql = 'SELECT id FROM platform_artifacts WHERE dataset=?';
  for (const [key, column] of [['applicationId','application'],['installationId','installation'],['output','output'],['project','project'],['status','status']]) {
    if (query[key] !== undefined) { values.push(identifier(query[key])); sql += ` AND ${column}=?`; }
  }
  if (query.tag !== undefined) {values.push(identifier(query.tag)); sql += " AND EXISTS(SELECT 1 FROM json_each(body,'$.tags') WHERE value=?)";}
  values.push(limit); sql += ' ORDER BY sequence DESC LIMIT ?';
  return {sql, values};
}
async function artifact(db: Database, dataset: string, id: string): Promise<any> {
  const row = await db.prepare('SELECT body,sequence FROM platform_artifacts WHERE dataset=? AND id=?').bind(dataset, identifier(id)).first<{body: string; sequence: number}>();
  if (!row) throw new Error('artifact not found or not authorized');
  return {...JSON.parse(row.body), sequence:row.sequence};
}
async function artifacts(db: Database, dataset: string, ids: string[]): Promise<any[]> {
  return Promise.all(ids.map(id => artifact(db, dataset, id)));
}
export async function execute(db: Database, principal: Principal, input: unknown): Promise<unknown> {
  const request = object(input);
  if (encoder.encode(JSON.stringify(request)).length > 65536) throw new Error('platform request exceeds 64 KiB');
  identifier(principal.applicationId); identifier(principal.installationId);
  const alias = identifier(request.dataset), action = identifier(request.action);
  const binding = alias === 'private' ? null : Object.hasOwn(principal.datasets || {}, alias) ? principal.datasets![alias] : undefined;
  if (alias !== 'private' && !binding) throw new Error('dataset not granted');
  if (binding && typeof binding.write !== 'boolean') throw new Error('dataset grant must specify a boolean write permission');
  const dataset = alias === 'private' ? `private:${principal.installationId}` : `shared:${identifier(binding!.id)}`;
  if (request.expectedScope !== undefined) {
    const expected = object(request.expectedScope);
    if (expected.applicationId !== principal.applicationId || expected.installationId !== principal.installationId || expected.datasetId !== dataset) throw new Error('remote dataset scope does not match the configured installation and dataset');
  }
  if (binding && !binding.write && !['record.get','artifact.get','artifact.list','artifact.resolve','artifact.lineage'].includes(action)) throw new Error('dataset is read-only');
  if (action.startsWith('record.')) {
    const key = identifier(request.key);
    if (action === 'record.get') {
      const row = await db.prepare('SELECT value,version,deleted FROM platform_records WHERE dataset=? AND key=?').bind(dataset,key).first<{value:string; version:number; deleted:number}>();
      return row ? {...row,value:JSON.parse(row.value),deleted:!!row.deleted} : null;
    }
    if (!['record.put','record.delete'].includes(action)) throw new Error('unknown platform operation');
    const expected = request.expectedVersion;
    if (expected !== null && (!Number.isSafeInteger(expected) || expected < 1)) throw new Error('expectedVersion is required (null to create)');
    if (expected === Number.MAX_SAFE_INTEGER) throw new Error('version exhausted');
    if (action === 'record.put' && !Object.hasOwn(request,'value')) throw new Error('value required');
    const value = action === 'record.delete' ? null : request.value, deleted = action === 'record.delete';
    const row = expected === null
      ? await db.prepare('INSERT INTO platform_records(dataset,key,value,version,deleted) VALUES(?,?,?,1,?) ON CONFLICT(dataset,key) DO NOTHING RETURNING version').bind(dataset,key,JSON.stringify(value),+deleted).first<{version:number}>()
      : await db.prepare('UPDATE platform_records SET value=?,version=version+1,deleted=? WHERE dataset=? AND key=? AND version=? RETURNING version').bind(JSON.stringify(value),+deleted,dataset,key,expected).first<{version:number}>();
    if (!row) throw new Error('conflict: record version changed');
    return {value, version:row.version, deleted};
  }
  if (action === 'artifact.get') return artifact(db,dataset,request.id);
  if (action === 'artifact.list') {
    const {sql,values} = discovery(dataset,request.query);
    const {results} = await db.prepare(sql).bind(...values).all<{id:string}>();
    return artifacts(db,dataset,results.map(r => r.id));
  }
  if (action === 'artifact.publish') {
    const draft = object(request.draft);
    if (Object.keys(draft).some(k => !['output','mediaType','content','project','tags','status','inputs','executionId','idempotencyKey'].includes(k))) throw new Error('unknown artifact draft field');
    const output = identifier(draft.output), mediaType = identifier(draft.mediaType);
    const declared = Object.hasOwn(principal.outputs || {}, output) ? principal.outputs![output] : undefined;
    if (declared && declared.mediaType !== mediaType) throw new Error('artifact mediaType differs from declared output');
    if (!Object.hasOwn(draft,'content')) throw new Error('artifact content is required');
    for (const key of ['project','status','executionId','idempotencyKey']) if (draft[key] !== undefined) identifier(draft[key]);
    for (const key of ['tags','inputs']) if (draft[key] !== undefined) {
      if (!Array.isArray(draft[key]) || draft[key].length > 100) throw new Error('tags and inputs must be arrays of at most 100 identifiers');
      draft[key].forEach(identifier);
    }
    await artifacts(db,dataset,draft.inputs || []);
    const id = crypto.randomUUID().replaceAll('-',''), createdAt = Date.now();
    const body = {...draft,id,applicationId:principal.applicationId,installationId:principal.installationId,datasetId:dataset,createdAt,tags:draft.tags || [],inputs:draft.inputs || []};
    const fingerprint = canonical(draft);
    await db.prepare('INSERT INTO platform_artifacts(id,dataset,installation,application,output,project,status,created,body,idempotency,fingerprint) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(dataset,installation,idempotency) DO NOTHING')
      .bind(id,dataset,principal.installationId,principal.applicationId,output,draft.project ?? null,draft.status ?? null,createdAt,JSON.stringify(body),draft.idempotencyKey ?? null,fingerprint).run();
    if (draft.idempotencyKey !== undefined) {
      const row = await db.prepare('SELECT id,fingerprint FROM platform_artifacts WHERE dataset=? AND installation=? AND idempotency=?').bind(dataset,principal.installationId,draft.idempotencyKey).first<{id:string;fingerprint:string}>();
      if (!row || row.fingerprint !== fingerprint) throw new Error('conflict: idempotency key reused for a different artifact');
      return artifact(db,dataset,row.id);
    }
    return artifact(db,dataset,id);
  }
  if (action === 'artifact.resolve') {
    const execution = identifier(request.executionId), slot = identifier(request.slot), reference = object(request.reference);
    const encoded = canonical(reference);
    let insert: Statement;
    if (reference.kind === 'permanent') {
      if (!Array.isArray(reference.ids) || !reference.ids.length || reference.ids.length > 100) throw new Error('permanent reference needs 1–100 IDs');
      reference.ids.forEach(identifier); await artifacts(db,dataset,reference.ids);
      insert = db.prepare('INSERT INTO platform_resolutions(dataset,installation,execution,slot,reference,ids) VALUES(?,?,?,?,?,?) ON CONFLICT DO NOTHING').bind(dataset,principal.installationId,execution,slot,encoded,JSON.stringify(reference.ids));
    } else if (reference.kind === 'dynamic') {
      const {sql,values} = discovery(dataset,reference.query);
      // Selection and snapshot publication happen in ONE statement, including empty results.
      insert = db.prepare(`INSERT INTO platform_resolutions(dataset,installation,execution,slot,reference,ids) SELECT ?,?,?,?,?,json_group_array(id) FROM (${sql}) WHERE true ON CONFLICT DO NOTHING`).bind(dataset,principal.installationId,execution,slot,encoded,...values);
    } else throw new Error('unknown artifact reference kind');
    const results = await db.batch<{reference:string;ids:string}>([insert,db.prepare('SELECT reference,ids FROM platform_resolutions WHERE dataset=? AND installation=? AND execution=? AND slot=?').bind(dataset,principal.installationId,execution,slot)]);
    const saved = results[1].results[0];
    if (saved.reference !== encoded) throw new Error('conflict: execution input slot already has a different reference');
    return artifacts(db,dataset,JSON.parse(saved.ids));
  }
  if (action === 'artifact.lineage') {
    const value = await artifact(db,dataset,request.id);
    const {results} = await db.prepare("SELECT id FROM platform_artifacts WHERE dataset=? AND EXISTS(SELECT 1 FROM json_each(body,'$.inputs') WHERE value=?) ORDER BY sequence DESC LIMIT 100").bind(dataset,value.id).all<{id:string}>();
    return {artifact:value,inputs:await artifacts(db,dataset,value.inputs),dependents:await artifacts(db,dataset,results.map(r=>r.id))};
  }
  throw new Error('unknown platform operation');
}

/** Authentication is host-controlled; bodies cannot override a principal or grants. */
export default {
  async fetch(request: Request, env: Environment): Promise<Response> {
    if (request.method !== 'POST') return Response.json({error:'POST required'},{status:405});
    const authorization = request.headers.get('authorization') || '';
    let principals: Record<string, any>;
    try {principals = object(JSON.parse(env.PRINCIPALS));} catch {return Response.json({error:'capability service is not configured'},{status:503});}
    try {
      const token = authorization.startsWith('Bearer ') ? authorization.slice(7) : '';
      if (!token || !Object.hasOwn(principals,token)) return Response.json({error:'unauthorized'},{status:403});
      // Bound the body while streaming, even without Content-Length.
      const reader = request.body?.getReader();
      if (!reader) throw new Error('request body required');
      let size = 0; const chunks: Uint8Array[] = [];
      for (;;) {
        const {done,value} = await reader.read(); if (done) break;
        size += value.length; if (size > 70000) {await reader.cancel(); return Response.json({error:'request too large'},{status:413});} chunks.push(value);
      }
      const bytes = new Uint8Array(size); let offset = 0;
      for (const chunk of chunks) {bytes.set(chunk,offset);offset += chunk.length;}
      const body = JSON.parse(new TextDecoder().decode(bytes));
      // Cloud adapter sends storage envelopes; remote local hosts send platform requests.
      if (body.operation !== undefined) {
        if (body.operation === 'identity') return Response.json({value:{applicationId:principals[token].applicationId,installationId:principals[token].installationId,target:'cloudflare'}});
        if (body.operation === 'platform') return Response.json({value:await execute(env.DB,principals[token],body.request)});
        return Response.json({value:await legacyStorage(env.DB,principals[token],body)});
      }
      return Response.json({value:await execute(env.DB,principals[token],body)});
    } catch (error) {
      const message = error instanceof Error ? error.message : 'dataset operation failed';
      // Do not echo provider errors containing SQL, credentials, or connection details.
      if (message.includes('quota exceeded')) return Response.json({error:'dataset quota exceeded'},{status:413});
      if (/D1_|SQLITE_|database|binding/i.test(message)) return Response.json({error:'dataset unavailable; retry without changing expectedVersion or idempotencyKey'},{status:503});
      return Response.json({error:message},{status:message.startsWith('conflict:') ? 409 : 400});
    }
  },
};
async function legacyStorage(db:Database, principal:Principal, request:Record<string,any>):Promise<unknown> {
  if (!['config','data'].includes(request.namespace) || !['get','set','delete'].includes(request.operation)) throw new Error('invalid storage operation');
  const dataset = `legacy:${principal.installationId}:${request.namespace}`, key = identifier(request.key);
  if (request.operation === 'get') {
    const row = await db.prepare('SELECT value FROM platform_records WHERE dataset=? AND key=?').bind(dataset,key).first<{value:string}>();
    return row ? JSON.parse(row.value) : null;
  }
  if (request.operation === 'delete') {await db.prepare('DELETE FROM platform_records WHERE dataset=? AND key=?').bind(dataset,key).run();return null;}
  const encoded = JSON.stringify(request.value ?? null);
  if (encoder.encode(encoded).length > 65536) throw new Error('storage value exceeds 64 KiB');
  await db.prepare('INSERT INTO platform_records(dataset,key,value,version,deleted) VALUES(?,?,?,1,0) ON CONFLICT(dataset,key) DO UPDATE SET value=excluded.value,version=version+1').bind(dataset,key,encoded).run();
  return request.value ?? null;
}
