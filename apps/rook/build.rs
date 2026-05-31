use std::path::Path;
use std::process::Command;

fn main() {
    let dashboard_dir = Path::new("dashboard");
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

    println!("cargo:rerun-if-changed=dashboard/dist");
    println!("cargo:rerun-if-changed=dashboard/src");
    println!("cargo:rerun-if-changed=dashboard/vite.config.ts");
    println!("cargo:rerun-if-changed=dashboard/package.json");
}
