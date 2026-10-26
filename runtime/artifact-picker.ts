/** Small DOM picker for choosing durable artifact references. */

export interface Artifact {
  id: string;
  sequence?: number;
  applicationId?: string;
  installationId?: string;
  datasetId?: string;
  createdAt?: number;
  output?: string;
  mediaType?: string;
  project?: string;
  tags?: string[];
  status?: string;
}

export interface ArtifactQuery {
  applicationId?: string;
  output?: string;
  project?: string;
  tag?: string;
  status?: string;
  limit?: number;
}

export type ArtifactReference =
  | { kind: "dynamic"; query: ArtifactQuery }
  | { kind: "permanent"; ids: string[] };

export interface ArtifactPickerApi {
  list(query: ArtifactQuery): Promise<Artifact[]>;
  get(id: string): Promise<Artifact>;
}

export interface ArtifactPickerHandle { destroy(): void }

/** Mounts a bounded, host-API-backed artifact picker into `element`. */
export function mountArtifactPicker(
  element: HTMLElement,
  artifacts: ArtifactPickerApi,
  onSelect: (reference: ArtifactReference) => void,
): ArtifactPickerHandle {
  let alive = true;
  let revision = 0;
  const root = document.createElement("section");
  root.setAttribute("aria-label", "Artifact picker");
  const heading = document.createElement("h2");
  heading.textContent = "Choose an artifact";
  root.append(heading);

  const field = (labelText: string, name: string): HTMLInputElement => {
    const label = document.createElement("label");
    label.textContent = labelText;
    const input = document.createElement("input");
    input.name = name;
    input.type = "text";
    label.append(input);
    root.append(label);
    return input;
  };
  const applicationId = field("Application ID", "applicationId");
  const output = field("Output", "output");
  const project = field("Project", "project");
  const tag = field("Tag", "tag");
  const status = field("Status", "status");
  const count = document.createElement("select");
  count.setAttribute("aria-label", "Dynamic reference window");
  for (const [value, title] of [["1", "Latest"], ["5", "Last 5"], ["20", "Last 20"]]) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = title;
    count.append(option);
  }
  root.append(count);

  const idInput = field("Pin explicit artifact ID", "artifactId");
  const previewButton = document.createElement("button");
  previewButton.type = "button";
  previewButton.textContent = "Preview query";
  const pinButton = document.createElement("button");
  pinButton.type = "button";
  pinButton.textContent = "Preview pinned artifact";
  const saveButton = document.createElement("button");
  saveButton.type = "button";
  saveButton.textContent = "Save selected reference";
  saveButton.disabled = true;
  const results = document.createElement("ul");
  const message = document.createElement("p");
  message.setAttribute("role", "status");
  root.append(previewButton, pinButton, results, saveButton, message);
  element.append(root);

  let selection: { reference: ArtifactReference; label: string } | undefined;
  const invalidate = () => {
    revision++;
    selection = undefined;
    results.replaceChildren();
    saveButton.disabled = true;
    message.textContent = "Inputs changed. Preview again to select artifacts.";
  };
  for (const input of [applicationId, output, project, tag, status, idInput]) {
    input.addEventListener("input", invalidate);
    input.addEventListener("change", invalidate);
  }
  count.addEventListener("input", invalidate);
  count.addEventListener("change", invalidate);
  const query = (): ArtifactQuery => {
    const q: ArtifactQuery = { limit: Math.max(1, Math.min(100, Number(count.value) || 1)) };
    for (const [key, input] of [["applicationId", applicationId], ["output", output], ["project", project], ["tag", tag], ["status", status]] as const) {
      const value = input.value.trim();
      if (value) q[key] = value;
    }
    return q;
  };
  const show = (items: Artifact[], label: string, reference: ArtifactReference, current: number) => {
    if (!alive) return;
    results.replaceChildren();
    for (const item of items) {
      const li = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      const createdAt = typeof item.createdAt === "number" && Number.isFinite(item.createdAt)
        ? new Date(item.createdAt).toLocaleString()
        : undefined;
      const metadata = [item.mediaType, item.project, item.status, item.tags?.join(", "), createdAt]
        .filter((part): part is string => Boolean(part));
      button.textContent = `${item.output || "Artifact"} · ${item.id}${metadata.length ? ` · ${metadata.join(" · ")}` : ""}`;
      button.addEventListener("click", () => {
        if (!alive || current !== revision) return;
        selection = { reference, label };
        saveButton.disabled = false;
        message.textContent = `Selected ${label}: ${item.id}`;
      });
      li.append(button);
      results.append(li);
    }
    message.textContent = items.length ? `${items.length} result${items.length === 1 ? "" : "s"} from ${label}.` : `No artifacts found for ${label}.`;
    if (!items.length) {
      selection = undefined;
      saveButton.disabled = true;
    }
  };
  const run = async (kind: "dynamic" | "permanent") => {
    const current = ++revision;
    selection = undefined;
    results.replaceChildren();
    saveButton.disabled = true;
    message.textContent = "Loading artifacts…";
    try {
      if (kind === "dynamic") {
        const q = query();
        const items = await artifacts.list(q);
        if (alive && current === revision) show(items, `dynamic query (latest ${q.limit})`, { kind, query: q }, current);
      } else {
        const id = idInput.value.trim();
        if (!id) throw new Error("Enter an artifact ID to pin.");
        const item = await artifacts.get(id);
        if (alive && current === revision) show([item], `permanent artifact ${id}`, { kind, ids: [item.id] }, current);
      }
    } catch (error) {
      if (alive && current === revision) message.textContent = error instanceof Error ? error.message : String(error);
    }
  };
  previewButton.addEventListener("click", () => { void run("dynamic"); });
  pinButton.addEventListener("click", () => { void run("permanent"); });
  saveButton.addEventListener("click", () => {
    if (!selection || !alive) return;
    onSelect(selection.reference);
    message.textContent = `Saved ${selection.label}.`;
  });

  return {
    destroy() {
      alive = false;
      revision++;
      root.remove();
      selection = undefined;
    },
  };
}
