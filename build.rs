use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=web/dist");

    let dist = Path::new("web/dist");
    if dist.exists() {
        return;
    }

    fs::create_dir_all(dist).expect("failed to create web/dist fallback directory");
    fs::write(
        dist.join("index.html"),
        r#"<!doctype html><html><head><meta charset="utf-8"><title>DiffScope</title></head><body>Frontend not built. Run npm run build in web/ for the full UI.</body></html>"#,
    )
    .expect("failed to write web/dist fallback index.html");
}
