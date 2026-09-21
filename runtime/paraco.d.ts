/** Draft public contracts for the v1 manifest and embedded Deno adapter. */
export interface ParacoManifest {
  /** Omit for compatibility with existing v1 manifests, or set to 1. */
  schemaVersion?: 1;
  name: string;
  entrypoint: string;
  capabilities: Array<"ai">;
}

export interface AiCompletionRequest {
  provider?: string;
  model?: string;
  prompt: string;
}

export interface AiCompletionResponse {
  selection: { provider: string; model: string };
  text: string;
}


export interface AppContext {
  /** The public mount prefix; it always begins and ends with '/'. */
  basePath: string;
  /** Present only when the host grants the app the local AI capability. */
  ai?: { complete(request: AiCompletionRequest): Promise<AiCompletionResponse> };
}

export interface ParacoApp {
  fetch(request: Request, context: AppContext): Response | Promise<Response>;
}

declare const app: ParacoApp;
export default app;
