import type { ParacoApp, AppContext, ParacoManifest } from './paraco';
import {datasetClient} from './platform-client.js';

type Capability = 'ai' | 'storage' | 'tasks';
interface Environment {
  PARACO?: {fetch(request: Request): Promise<Response>};
}
/** A deployment-owned service binding supplies equivalent scoped capabilities.
 * The service must authenticate its caller, enforce grants, and scope storage;
 * application-supplied capability names never authorize access on their own.
 */
export function cloudflare(app: ParacoApp, capabilities: Capability[] = [], manifest?: ParacoManifest) {
  if (capabilities.includes('tasks')) throw new Error('Cloudflare task scheduling is not supported');
  if (manifest?.targets && !manifest.targets.includes('cloudflare')) throw new Error('Application does not support cloudflare');
  if (manifest && (manifest.capabilities.includes('tasks') || manifest.capabilities.some(c => !capabilities.includes(c)))) throw new Error('Required capability is unsupported or missing');
  return {
    async fetch(request: Request, env: Environment): Promise<Response> {
      if (capabilities.length && !env.PARACO) {
        return new Response('Missing deployment capability service binding PARACO', {status:503});
      }
      const call = async (name: Capability, body: unknown) => {
        const response = await env.PARACO!.fetch(new Request(`https://paraco/${name}`, {
          method:'POST', headers:{'content-type':'application/json'}, body:JSON.stringify(body),
        }));
        const value = await response.json() as {error?:string; result?:any; value?:any};
        if (!response.ok || value.error) throw new Error(value.error || 'Capability service failed');
        return name === 'ai' ? value.result : value.value;
      };
      const context: AppContext = {basePath:'/'};
      if (capabilities.includes('ai')) context.ai = Object.freeze({complete: (request:unknown) => call('ai',{request})});
      if (capabilities.includes('storage')) {
        context.identity = Object.freeze(await call('storage',{operation:'identity'}));
        context.datasets = Object.freeze({open:(alias = 'private') => datasetClient(request => call('storage',{operation:'platform',request}),alias)});
        context.artifacts = context.datasets.open().artifacts;
      }
      if (capabilities.includes('storage')) for (const namespace of ['config','data'] as const) {
        context[namespace] = Object.freeze({
          get:(key:string) => call('storage',{namespace,key,operation:'get'}),
          set:(key:string,value:unknown) => call('storage',{namespace,key,value,operation:'set'}),
          delete:(key:string) => call('storage',{namespace,key,operation:'delete'}),
        });
      }

      return app.fetch(request,Object.freeze(context));
    },
  };
}
