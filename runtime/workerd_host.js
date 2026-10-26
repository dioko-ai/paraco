import app from './app.js';
import {datasetClient} from './platform-client.js';
export default {
  async fetch(request, env) {
    const settings = env.SETTINGS;
    if (typeof app?.fetch !== 'function') { console.error('app default export must provide fetch(request, context)'); return new Response('Invalid application contract',{status:500}); }
    if (request.headers.get('x-paraco-ready') === settings.nonce) return new Response('ready');
    if (settings.backendToken && request.headers.get('x-paraco-internal') !== settings.backendToken) return new Response('Forbidden', {status:403});
    const headers = new Headers(request.headers);
    headers.delete('x-paraco-internal'); headers.delete('x-paraco-ready');
    const url = new URL(request.url);
    if (settings.backendToken) url.host = headers.get('host');
    request = new Request(url, new Request(request, {headers}));
    const call = async (name, body) => {
      const reply = await env.HOST.fetch(`http://capability/${name}`, {method:'POST', headers:{'content-type':'application/json',authorization:`Bearer ${settings.hostToken}`} , body:JSON.stringify(body)});
      const value = await reply.json();
      if (!reply.ok || value.error) throw new Error(value.error || 'Capability failed');
      return name === 'ai' ? value.result : value.value;
    };
    const context = {basePath:settings.basePath, identity:Object.freeze(settings.identity)};
    if (settings.storage) {
      context.datasets = Object.freeze({open: (alias = 'private') => datasetClient(request => call('storage', {operation:'platform', request}), alias)});
      context.artifacts = context.datasets.open().artifacts;
    }
    if (settings.ai) context.ai = Object.freeze({complete: request => call('ai', {request})});
    if (settings.storage) for (const namespace of ['config','data']) context[namespace] = Object.freeze({
      get:key => call('storage',{namespace,key,operation:'get'}),
      set:(key,value) => call('storage',{namespace,key,value,operation:'set'}),
      delete:key => call('storage',{namespace,key,operation:'delete'})
    });
    if (settings.system) context.system = Object.freeze({snapshot:() => call('system',{})});
    if (settings.tasks) context.tasks = Object.freeze({command:(action,body={}) => call('tasks',{action,body})});
    try {
      if (typeof app?.fetch !== 'function') throw new Error('App must export fetch(request, context)');
      return await app.fetch(request,Object.freeze(context));
    } catch (error) { console.error('Paraco app request failed:',error); return new Response('Internal Server Error',{status:500}); }
  }
};
