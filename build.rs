use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    let commit =
        git_output(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let full_commit = git_output(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let dirty = Command::new("git")
        .args(["diff", "--quiet", "--ignore-submodules", "HEAD", "--"])
        .status()
        .map(|status| !status.success())
        .unwrap_or(false);
    let build = if dirty {
        format!("{}-dirty", commit)
    } else {
        commit
    };

    println!("cargo:rustc-env=AIC_FLASH_BUILD={}", build);
    println!("cargo:rustc-env=AIC_FLASH_COMMIT={}", full_commit);
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}
