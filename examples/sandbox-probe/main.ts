type Status = 'PASS' | 'FAIL' | 'UNVERIFIED';
type Check = { id: string; status: Status; detail: string };

// These checks describe what this guest can observe, not the host's OS isolation.
export async function probeEnvironment(env: any, context: any, fixture?: string): Promise<any> {
  const checks: Check[] = [];
  const check = (id: string, exposed: boolean, detail: string) => checks.push({
    id, status: exposed ? 'FAIL' : 'PASS', detail: exposed ? `${detail} exposed` : `${detail} not exposed to this guest`,
  });
  check('node', typeof env.process !== 'undefined' || typeof env.require !== 'undefined' || typeof env.module !== 'undefined', 'Node.js globals');
  check('deno', typeof env.Deno !== 'undefined' || typeof env.Bun !== 'undefined', 'Deno or Bun globals');
  check('filesystem', typeof env.process?.getBuiltinModule === 'function' || typeof env.Deno?.readFile === 'function' || typeof env.Bun?.file === 'function', 'Ambient filesystem API');
  check('subprocess', typeof env.process?.getBuiltinModule === 'function' || typeof env.Deno?.Command === 'function' || typeof env.Bun?.spawn === 'function', 'Ambient subprocess API');
  const exposed = ['ai', 'storage', 'tasks', 'system', 'datasets', 'artifacts', 'config', 'data'].filter(key => context?.[key] != null);
  check('capabilities', exposed.length > 0, exposed.length ? `Unexpected capabilities: ${exposed.join(', ')}` : 'Undeclared capabilities');
  try {
    const escaped = env.Function('return typeof process !== "undefined" || typeof require !== "undefined" || typeof Deno !== "undefined"')();
    check('dynamic-code', !!escaped, 'Host globals through dynamic code');
  } catch (error) {
    checks.push(error instanceof EvalError
      ? {id: 'dynamic-code', status: 'PASS', detail: 'Dynamic evaluation was denied in this guest.'}
      : {id: 'dynamic-code', status: 'UNVERIFIED', detail: 'Dynamic evaluation failed for an unknown reason.'});
  }
  let url: URL | undefined;
  try {
    if (fixture) {
      const parsed = new URL(fixture);
      if (parsed.protocol === 'http:' && parsed.hostname === '127.0.0.1' && parsed.port && !parsed.username && !parsed.password && !parsed.hash) url = parsed;
    }
  } catch { /* Invalid fixture is reported below. */ }
  if (fixture && !url) {
    checks.push({id: 'network', status: 'UNVERIFIED', detail: 'Fixture must be a controlled http://127.0.0.1:port URL.'});
  } else if (url) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 2000);
    try {
      await env.fetch(url.toString(), {signal: controller.signal, redirect: 'error'});
      checks.push({id: 'network', status: 'FAIL', detail: 'A direct request received a response from the loopback fixture.'});
    } catch {
      checks.push({id: 'network', status: 'UNVERIFIED', detail: 'The request failed or timed out. This does not establish why or prove that networking is blocked.'});
    } finally { clearTimeout(timer); }
  }
  const required = ['containment-mode', ...['native', 'compiler'].flatMap(prefix => ['control', 'read', 'write', 'readonly', 'network', 'exec', 'environment'].map(id => `${prefix}-${id}`))];
  const evidence = context?.containmentEvidence;
  const valid = evidence && evidence.schemaVersion === 1 && typeof evidence.backend === 'string' &&
    Array.isArray(evidence.limitations) && evidence.limitations.every((item: unknown) => typeof item === 'string') &&
    Array.isArray(evidence.checks) && evidence.checks.every((item: any) => item && typeof item.id === 'string' &&
      ['PASS', 'FAIL', 'UNVERIFIED'].includes(item.status) && typeof item.detail === 'string') &&
    new Set(evidence.checks.map((item: any) => item.id)).size === evidence.checks.length &&
    required.every(id => evidence.checks.some((item: any) => item.id === id));
  const hostChecks: Check[] = valid ? evidence.checks : [];
  const hostEvidence = valid
    ? {status: hostChecks.some(item => item.status === 'FAIL') ? 'FAIL' : hostChecks.every(item => item.status === 'PASS') ? 'PASS' : 'UNVERIFIED', backend: evidence.backend, checks: hostChecks, limitations: evidence.limitations}
    : {status: 'UNVERIFIED', backend: '', checks: [], limitations: ['Trusted host evidence is missing or invalid.']};
  const all = [...checks, ...hostChecks];
  const status = all.some(item => item.status === 'FAIL') ? 'FAIL' : hostEvidence.status === 'PASS' && all.every(item => item.status === 'PASS') ? 'PASS' : 'UNVERIFIED';
  return {status, checks, hostEvidence};
}

const page = `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Sandbox probe</title><style>body{font:16px system-ui;max-width:800px;margin:3rem auto;padding:0 1rem;color:#182132}input{width:min(100%,30rem);padding:.5rem}button{padding:.5rem}li{margin:1rem 0}strong{margin-right:.5rem}pre{white-space:pre-wrap}.PASS{color:#087443}.FAIL{color:#b91c1c}.UNVERIFIED{color:#805400}</style><h1>Sandbox probe</h1><p>This report describes observable guest behavior. It cannot prove host OS isolation. Use only a controlled loopback fixture.</p><form><label>Fixture URL (optional) <input name="fixture" placeholder="http://127.0.0.1:1234/"></label> <button>Run probes</button></form><p id="summary">Running…</p><h2>Guest checks</h2><ul id="checks"></ul><h2>Host containment checks</h2><p id="host"></p><ul id="host-checks"></ul><ul id="limitations"></ul><script>
const form=document.querySelector('form'), summary=document.querySelector('#summary'), list=document.querySelector('#checks'), host=document.querySelector('#host'),hostChecks=document.querySelector('#host-checks'),limitations=document.querySelector('#limitations');
async function run(){summary.className='UNVERIFIED';summary.textContent='Running…';list.replaceChildren();hostChecks.replaceChildren();limitations.replaceChildren();host.textContent='';try{const query=new URLSearchParams(new FormData(form));const response=await fetch(new URL('api/report?'+query,document.baseURI));if(!response.ok)throw Error('Report request failed');const report=await response.json();summary.textContent='Overall: '+report.status;summary.className=report.status;for(const check of report.checks){const li=document.createElement('li'),strong=document.createElement('strong');strong.textContent=check.status;strong.className=check.status;li.append(strong,document.createTextNode(check.id+': '+check.detail));list.append(li)}host.textContent=report.hostEvidence.status+(report.hostEvidence.backend?' — '+report.hostEvidence.backend:'');host.className=report.hostEvidence.status;for(const check of report.hostEvidence.checks){const li=document.createElement('li'),strong=document.createElement('strong');strong.textContent=check.status;strong.className=check.status;li.append(strong,document.createTextNode(check.id+': '+check.detail));hostChecks.append(li)}for(const text of report.hostEvidence.limitations){const li=document.createElement('li');li.textContent=text;limitations.append(li)}}catch(e){summary.className='UNVERIFIED';summary.textContent='UNVERIFIED: '+e.message}}form.addEventListener('submit',e=>{e.preventDefault();run()});run();</script></html>`;

export default {
  async fetch(request: Request, context: object): Promise<Response> {
    const url = new URL(request.url);
    if (request.method !== 'GET') return new Response('Method not allowed', {status: 405});
    if (url.pathname === '/api/report') return Response.json(await probeEnvironment(globalThis, context, url.searchParams.get('fixture') || undefined), {headers: {'Cache-Control': 'no-store'}});
    if (url.pathname === '/') return new Response(page, {headers: {'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store'}});
    return new Response('Not found', {status: 404});
  },
};
