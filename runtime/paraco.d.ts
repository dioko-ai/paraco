/** Draft public contracts for the v1 manifest and portable workerd adapter. */
export interface ParacoManifest {
  /** Omit for compatibility with existing v1 manifests, or set to 1. */
  schemaVersion?: 1;
  name: string;
  entrypoint: string;
  targets?: Array<"local" | "cloudflare">;
  outputs?: Record<string, {mediaType: string; schema?: unknown}>;
  capabilities: Array<"ai" | "storage" | "tasks" | "system">;
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


export interface ScopedStorage {
  get(key: string): Promise<unknown>;
  set(key: string, value: unknown): Promise<unknown>;
  delete(key: string): Promise<unknown>;
}
export interface AppContext {
  /** Read-only local system metrics; requires the system manifest capability. */
  system?: {snapshot(): Promise<SystemSnapshot>};
  identity?: Readonly<{applicationId: string; installationId: string; target: "local" | "cloudflare"}>;
  /** Versioned private or explicitly host-granted shared datasets; requires storage. */
  datasets?: {open(alias?: string): Dataset};
  /** Convenience alias for the private dataset artifact registry. */
  artifacts?: Artifacts;
  /** Explicit host-granted task authoring, scoped to this deployment. */
  tasks?: ScopedTasks;
  /** Deployment-scoped draft configuration and application data; requires storage. */
  config?: ScopedStorage;
  data?: ScopedStorage;
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

/** Draft task APIs. The host binds all operations to the calling deployment. */
export type TaskSchedule =
  | { kind: "manual" }
  | { kind: "once"; at: number }
  | { kind: "cron"; expression: string; timezone: string };
export interface TaskDraft {
  job_id?: string;
  name: string;
  source: string;
  schedule: TaskSchedule;
  timeout_ms?: number;
  network?: string[];
  notify_success?: boolean;
  notify_failure?: boolean;
}
export interface TaskExecution {
  deployment_id: string;
  job_id: string;
  revision_id: string;
  run_id: string;
}
export interface TaskContext {
  readonly execution: Readonly<TaskExecution>;
  log(...values: unknown[]): void;
  /** Durable inbox event; no secret/full-log desktop payloads. */
  notify(message: string): void;
}
export type ParacoTask = (context: Readonly<TaskContext>) => void | Promise<void>;
export interface SavedTaskDraft {
  revision_id: string;
  job_id: string;
  digest: string;
  body: TaskDraft;
}
export interface ScopedTasks {
  command(action: "draft", body: TaskDraft): Promise<SavedTaskDraft>;
  command(action: "publish", body: { revision_id: string; expected_current_revision: string | null }): Promise<{ job_id: string; revision_id: string; ok: true }>;
  command(action: "run", body: { job_id: string }): Promise<{ run_id: string }>;
  command(action: "archive" | "unarchive", body: { job_id: string }): Promise<unknown>;
  command(action: "cancel", body: { run_id: string }): Promise<unknown>;
  command(action: "preview", body: { schedule: TaskSchedule; after?: number }): Promise<{ times: number[] }>;
  command(action: "list" | "status" | "notifications", body?: object): Promise<unknown>;
  command(action: "revisions" | "history", body: { job_id: string }): Promise<unknown>;
  command(action: "revision", body: { revision_id: string }): Promise<unknown>;
  command(action: "logs", body: { run_id: string }): Promise<unknown>;
  command(action: "read_notification", body: { id: string }): Promise<unknown>;
}

/** A deleted record retains its version so stale writers cannot recreate it. */
export interface VersionedRecord { value: unknown; version: number; deleted: boolean }
export interface ArtifactDraft {
  output: string; mediaType: string; content: unknown;
  project?: string; tags?: string[]; status?: string;
  inputs?: string[]; executionId?: string; idempotencyKey?: string;
}
export interface Artifact extends ArtifactDraft {
  id: string; sequence: number; applicationId: string; installationId: string;
  datasetId: string; createdAt: number; tags: string[]; inputs: string[];
}
export interface ArtifactQuery {
  applicationId?: string; installationId?: string; output?: string;
  project?: string; tag?: string; status?: string; limit?: number;
}
export type ArtifactReference = {kind: "permanent"; ids: string[]} | {kind: "dynamic"; query: ArtifactQuery};
export interface Artifacts {
  publish(draft: ArtifactDraft): Promise<Artifact>;
  get(id: string): Promise<Artifact>;
  list(query?: ArtifactQuery): Promise<Artifact[]>;
  /** First use snapshots permanent IDs; reuse of a slot cannot change its reference. */
  resolve(reference: ArtifactReference, execution: {executionId: string; slot: string}): Promise<Artifact[]>;
  lineage(id: string): Promise<{artifact: Artifact; inputs: Artifact[]; dependents: Artifact[]}>;
}
export interface Dataset {
  records: {
    get(key: string): Promise<VersionedRecord | null>;
    put(key: string, value: unknown, expectedVersion: number | null): Promise<VersionedRecord>;
    delete(key: string, expectedVersion: number | null): Promise<VersionedRecord>;
  };
  artifacts: Artifacts;
}

export interface SystemSnapshot {
  sampledAt: number; hostname: string; platform: string; cores: number;
  cpu: {total: number; idle: number};
  memory: {total: number; available: number; swapTotal: number; swapFree: number};
  load: number[]; uptime: number;
  ports: {address: string; port: number; protocol: string; state: string}[];
  warnings: string[];
}
