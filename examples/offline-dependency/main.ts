import { basename } from "./basename.ts";
export default { fetch() { return new Response(basename("/offline/dependency-ok")); } };
