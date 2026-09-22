import { basename } from "basename";
export default { fetch() { return new Response(basename('/offline/dependency-ok')); } };
