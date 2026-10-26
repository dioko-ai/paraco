/** One host-owned worker per installation. Expose ONLY as a service binding.
 * Downloaded apps receive PARACO pointing here, never TOKEN or SERVICE/DB.
 */
interface Environment {
  SERVICE: {fetch(request: Request): Promise<Response>};
  TOKEN: string;
}
export default {
  async fetch(request: Request, env: Environment): Promise<Response> {
    if (request.method !== 'POST' || new URL(request.url).pathname !== '/storage') {
      return Response.json({error:'unsupported capability'},{status:403});
    }
    // Ignore all caller headers, URL authority, and claimed identities.
    return env.SERVICE.fetch(new Request('https://platform/', {
      method:'POST', headers:{'content-type':'application/json',authorization:`Bearer ${env.TOKEN}`},body:request.body,
    }));
  },
};
