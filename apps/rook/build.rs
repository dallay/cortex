use std::path::Path;
use std::process::Command;

fn main() {
    // Emit rerun-if-changed unconditionally so Cargo knows when to rebuild
    println!("cargo:rerun-if-changed=dashboard/dist");
    println!("cargo:rerun-if-changed=dashboard/src");
    println!("cargo:rerun-if-changed=dashboard/vite.config.ts");
    println!("cargo:rerun-if-changed=dashboard/package.json");

    // Only build dashboard if node_modules/.bin/vite exists (i.e. deps are installed)
    // This allows `cargo check` to pass without running the full vite build
    let dashboard_dir = Path::new("dashboard");
    let vite_path = dashboard_dir.join("node_modules/.bin/vite");

    if vite_path.exists() {
        let status = Command::new("sh")
            .current_dir(dashboard_dir)
            .arg("-c")
            .arg("./node_modules/.bin/vite build")
            .status()
            .expect("failed to run dashboard build: sh or vite not found");

        if !status.success() {
            eprintln!("dashboard build failed with exit code: {}", status);
            std::process::exit(1);
        }
    } else {
        let profile = std::env::var("PROFILE").unwrap_or_default();
        if profile == "release" {
            eprintln!(
                "error: dashboard/node_modules/.bin/vite not found in release mode"
            );
            eprintln!("hint: run `pnpm install` in the repo root before building release"
            );
            std::process::exit(1);
        }
        println!(
            "cargo:warning=dashboard/node_modules/.bin/vite not found, skipping dashboard build"
        );
        println!("cargo:warning=hint: run `pnpm install` in the repo root to enable dashboard embedding");
    }
}
