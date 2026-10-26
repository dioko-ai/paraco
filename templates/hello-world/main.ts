const page = `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="theme-color" content="#101827">
  <title>Hello, world — Paraco</title>
  <style>
    :root { color-scheme: dark; font-family: ui-sans-serif, system-ui, sans-serif; }
    * { box-sizing: border-box; }
    body { min-height: 100vh; margin: 0; display: grid; place-items: center; padding: 2rem;
      color: #f3f6fc; background: radial-gradient(ellipse at 50% 0%, #253c60 0, #101827 58%); }
    main { width: min(100%, 620px); padding: clamp(2rem, 7vw, 4rem); border: 1px solid #ffffff20;
      border-radius: 28px; background: #152033cc; box-shadow: 0 24px 80px #0005; }
    .mark { display: grid; place-items: center; width: 58px; height: 58px; margin-bottom: 2rem;
      border-radius: 18px; color: #101827; background: #a7f3d0; font-size: 1.8rem; font-weight: 800; }
    .eyebrow { color: #a7f3d0; font-size: .78rem; font-weight: 700; letter-spacing: .15em; text-transform: uppercase; }
    h1 { margin: .7rem 0 1rem; font-size: clamp(2.5rem, 8vw, 4.5rem); letter-spacing: -.06em; line-height: 1; }
    p { max-width: 43ch; color: #b9c5d8; font-size: 1.1rem; line-height: 1.7; }
    a { display: inline-flex; align-items: center; gap: .6rem; margin-top: 1.25rem; padding: .85rem 1.15rem;
      border-radius: 12px; color: #101827; background: #a7f3d0; font-weight: 750; text-decoration: none; }
    a:hover { background: #c4ffe3; transform: translateY(-1px); }
    .note { margin-top: 2.5rem; padding-top: 1.25rem; border-top: 1px solid #ffffff20; color: #8190a7; font-size: .9rem; }
  </style>
</head>
<body>
  <main>
    <div class="mark" aria-hidden="true">P</div>
    <div class="eyebrow">Your first Paraco app</div>
    <h1>Hello,<br>world.</h1>
    <p>A small, self-contained app is up and running. Make it yours, then share it with the world.</p>
    <a href="/health">Check app health <span aria-hidden="true">↗</span></a>
    <div class="note">Built with curiosity. Ready for what comes next.</div>
  </main>
</body>
</html>`;

export function handleRequest(request: Request): Response {
  const { pathname } = new URL(request.url);
  if (pathname === "/health") {
    return Response.json({ status: "ok", name: "hello-world" });
  }
  if (pathname === "/" || pathname === "/index.html") {
    return new Response(page, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }
  return new Response("Not found", { status: 404 });
}

// Paraco owns the listener and calls this standard handler.
export default { fetch: handleRequest };
