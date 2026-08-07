use std::{env, fs, path::Path, process::Command};

fn command_output(command: &str, args: &[&str]) -> String {
    Command::new(command)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn main() {
    let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo");
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_owned());
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let rustc_version = command_output(&rustc, &["--version"]);
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned());

    println!("cargo:rustc-env=TG_BOT_RUSTC_VERSION={rustc_version}");
    println!("cargo:rustc-env=TG_BOT_TARGET={target}");

    let version_text = format!(
        "{}\nbuild-time: {}\ncommit: {}\ntarget: {}\nrustc: {}",
        version,
        command_output("date", &["-u", "+%Y-%m-%d %H:%M:%S UTC"]),
        command_output("git", &["rev-parse", "--short", "HEAD"]),
        target,
        rustc_version,
    );

    fs::write(Path::new(&out_dir).join("version.txt"), version_text)
        .expect("failed to write generated version information");
}
