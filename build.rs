fn main() {
    println!(
        "cargo:rustc-env=ELFVIS_BUILD_TIME={}",
        std::process::Command::new("date")
            .args(["-u", "+%Y-%m-%d %H:%M UTC"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "unknown".into())
    );
}
