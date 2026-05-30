use std::path::Path;
use std::process::Command;

fn main() {
    let dashboard_dir = Path::new("dashboard");
    let status = Command::new("bash")
        .current_dir(dashboard_dir)
        .arg("build.sh")
        .status()
        .expect("failed to run dashboard build script");

    if !status.success() {
        eprintln!("dashboard build failed with exit code: {}", status);
        std::process::exit(1);
    }

    println!("cargo:rerun-if-changed=dashboard/dist");
    println!("cargo:rerun-if-changed=dashboard/src");
    println!("cargo:rerun-if-changed=dashboard/vite.config.ts");
    println!("cargo:rerun-if-changed=dashboard/package.json");
}
