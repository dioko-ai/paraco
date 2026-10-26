import task from './app.js';
let used = false;
export default { async fetch(request, env) {
  if (used || request.method !== 'POST') return new Response('Conflict',{status:409});
  used = true;
  let notifications=0;
  const ctx=Object.freeze({execution:Object.freeze(env.SETTINGS),log:(...values)=>console.log(...values),notify:message=>{
    if (++notifications>100 || typeof message!=='string' || message.length>2048) throw new Error('Invalid or excessive notification');
    console.log('PARACO_NOTIFICATION:'+JSON.stringify({message}));
  }});
  try { if (typeof task!=='function') throw new Error('Task must export a default function'); await task(ctx); return new Response('done'); }
  catch(error) { console.error(error); return new Response('Task failed',{status:500}); }
}};
