use std::env;

fn main() {
    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".to_owned());
    println!("cargo:rustc-env=MISO_NATIVE_SCORE_CARGO_PROFILE={profile}");
}
