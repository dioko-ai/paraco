// Concrete storage example for cron drafts; this does not publish or execute jobs.
export default {
  async fetch(request: Request, context: any) {
    if (request.method === "PUT") {
      const draft = await request.json();
      if (typeof draft.source !== "string") return new Response("source required", {status: 400});
      await context.data.set("draft", draft);
      await context.config.set("timezone", draft.timezone ?? "UTC");
    }
    return Response.json({ draft: await context.data.get("draft"), timezone: await context.config.get("timezone") });
  },
};
