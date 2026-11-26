use std::collections::HashMap;
use std::path::PathBuf;

fn load_env_local_vars(keys: &[&str]) {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let env_path = manifest_dir.join("..").join(".env.local");
    println!("cargo:rerun-if-changed={}", env_path.display());

    let mut file_vars = HashMap::new();
    if let Ok(content) = std::fs::read_to_string(&env_path) {
        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let key = k.trim();
                let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
                if !val.is_empty() {
                    file_vars.insert(key.to_string(), val);
                }
            }
        }
    }

    for &key in keys {
        println!("cargo:rerun-if-env-changed={key}");
        let val = std::env::var(key)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| file_vars.get(key).cloned());
        if let Some(v) = val {
            println!("cargo:rustc-env={key}={v}");
        }
    }
}

fn main() {
    load_env_local_vars(&["GOOGLE_CLIENT_ID", "GOOGLE_CLIENT_SECRET"]);
    tauri_build::build();
}