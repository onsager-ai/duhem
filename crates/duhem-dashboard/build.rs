//! Ensure `web/dist/` exists before `rust-embed` reads it at compile
//! time. A plain `cargo build` (CI's fast Rust lane, a contributor
//! without Node) gets a placeholder index; building the SPA
//! (`npm ci && npm run build` under `web/`) replaces it with the real
//! bundle and triggers a rebuild via `rerun-if-changed`.

use std::fs;
use std::path::Path;

const PLACEHOLDER: &str = "\
<!doctype html>
<html>
  <head><meta charset=\"utf-8\"><title>Duhem dashboard</title></head>
  <body>
    <h1>Duhem dashboard</h1>
    <p>SPA bundle not built. Run <code>npm ci &amp;&amp; npm run build</code>
    under <code>crates/duhem-dashboard/web/</code> and rebuild, or use the
    JSON API under <code>/api/</code> directly.</p>
  </body>
</html>
";

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("web/dist");
    println!("cargo:rerun-if-changed=web/dist");
    if !dist.join("index.html").exists() {
        fs::create_dir_all(&dist).expect("create web/dist");
        fs::write(dist.join("index.html"), PLACEHOLDER).expect("write placeholder index.html");
    }
}
