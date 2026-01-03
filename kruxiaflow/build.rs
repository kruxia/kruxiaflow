use std::process::Command;

fn main() {
    // Capture build timestamp in ISO 8601 format
    // Use UTC time for consistency across different build environments
    let output = Command::new("date")
        .arg("-u")
        .arg("+%Y-%m-%dT%H:%M:%SZ")
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let build_date = String::from_utf8_lossy(&output.stdout);
            println!("cargo:rustc-env=BUILD_TIMESTAMP={}", build_date.trim());
        } else {
            // Fallback if date command fails
            println!("cargo:rustc-env=BUILD_TIMESTAMP=unknown");
        }
    } else {
        // Fallback if date command is not available
        println!("cargo:rustc-env=BUILD_TIMESTAMP=unknown");
    }

    // Capture git commit short hash
    let git_output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();

    if let Ok(output) = git_output {
        if output.status.success() {
            let git_hash = String::from_utf8_lossy(&output.stdout);
            println!("cargo:rustc-env=BUILD_GIT_HASH={}", git_hash.trim());
        } else {
            println!("cargo:rustc-env=BUILD_GIT_HASH=unknown");
        }
    } else {
        println!("cargo:rustc-env=BUILD_GIT_HASH=unknown");
    }

    // Capture git commit full hash
    let git_full_output = Command::new("git").args(["rev-parse", "HEAD"]).output();

    if let Ok(output) = git_full_output {
        if output.status.success() {
            let git_hash = String::from_utf8_lossy(&output.stdout);
            println!("cargo:rustc-env=BUILD_GIT_HASH_FULL={}", git_hash.trim());
        } else {
            println!("cargo:rustc-env=BUILD_GIT_HASH_FULL=unknown");
        }
    } else {
        println!("cargo:rustc-env=BUILD_GIT_HASH_FULL=unknown");
    }

    // Capture git branch
    let git_branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output();

    if let Ok(output) = git_branch_output {
        if output.status.success() {
            let git_branch = String::from_utf8_lossy(&output.stdout);
            println!("cargo:rustc-env=BUILD_GIT_BRANCH={}", git_branch.trim());
        } else {
            println!("cargo:rustc-env=BUILD_GIT_BRANCH=unknown");
        }
    } else {
        println!("cargo:rustc-env=BUILD_GIT_BRANCH=unknown");
    }

    // Rerun build script if git HEAD changes
    println!("cargo:rerun-if-changed=../.git/HEAD");
}
