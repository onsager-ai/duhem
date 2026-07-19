// A deliberately tiny "app" that reproduces the self-masking pattern from
// the README's proof story: the health endpoint always reports healthy,
// while the web front can be quietly broken. Both endpoints answer 200 —
// only the *content* of the front betrays the break, exactly like an
// empty nginx config falling back to a default page.
const http = require("http");

const port = Number(process.env.PORT || 8477);
const fixed = process.env.APP_FIXED === "1";

const send = (res, code, obj) => {
  res.writeHead(code, { "content-type": "application/json" });
  res.end(JSON.stringify(obj));
};

http
  .createServer((req, res) => {
    const path = (req.url || "/").split("?")[0];
    if (path === "/health") return send(res, 200, { status: "healthy" }); // always green
    if (path === "/")
      // Broken and fixed both answer 200; only `title` differs.
      return send(res, 200, { app: "acme", title: fixed ? "Acme Dashboard" : "Welcome to nginx!" });
    return send(res, 404, { error: "not found" });
  })
  .listen(port, "127.0.0.1", () => console.error(`app on ${port} (fixed=${fixed})`));
