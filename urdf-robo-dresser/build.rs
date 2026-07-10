//! Build script: tell Cargo to rebuild the binary whenever any file
//! under `web/` changes. The `include_dir!` macro embeds those files
//! into the compiled binary; without this hook, edits to JS / HTML /
//! CSS aren't visible to cargo's incremental builder and the cached
//! object file gets reused, so the embedded SPA stays stale.

use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=web");
    walk(Path::new("web"));
}

fn walk(dir: &Path) {
    let entries = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        // Print every entry — `cargo:rerun-if-changed` works on
        // directories, but listing each file makes the dependency
        // explicit and survives renames.
        if let Some(s) = p.to_str() {
            println!("cargo:rerun-if-changed={}", s);
        }
        if p.is_dir() {
            walk(&p);
        }
    }
}
