interface Context {
  ai: {
    complete(
      request: { prompt: string; provider?: string; model?: string },
    ): Promise<{
      selection: { provider: string; model: string };
      text: string;
    }>;
  };
}

export default {
  async fetch(_request: Request, context: Context): Promise<Response> {
    const result = await context.ai.complete({ prompt: "Hello" });
    return Response.json(result);
  },
};
