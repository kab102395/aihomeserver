use std::collections::HashMap;
use crate::coder::adapter::{
    AdapterManifest, FilesystemContract, LanguageAdapter, PackagingSpec, ToolchainCheck,
    VerificationLevels,
};

pub struct RustAdapter;

impl LanguageAdapter for RustAdapter {
    fn manifest(&self) -> AdapterManifest {
        let mut recipes = HashMap::new();
        recipes.insert("check_project".to_string(), "cargo check".to_string());
        recipes.insert("build_release".to_string(), "cargo build --release".to_string());
        recipes.insert("build_debug".to_string(), "cargo build".to_string());
        recipes.insert("format_check".to_string(), "cargo fmt --check".to_string());
        recipes.insert("lint".to_string(), "cargo clippy -- -D warnings".to_string());
        recipes.insert("test".to_string(), "cargo test".to_string());

        AdapterManifest {
            language: "rust".to_string(),
            aliases: vec!["rs".to_string(), "cargo".to_string()],
            project_markers: vec![
                "Cargo.toml".to_string(),
                "src/main.rs".to_string(),
                "src/lib.rs".to_string(),
            ],
            profiles: vec![
                "cli".to_string(),
                "terminal-game-crossterm".to_string(),
                "gui-macroquad".to_string(),
                "web-axum".to_string(),
                "web-actix".to_string(),
                "library".to_string(),
            ],
            toolchain_checks: vec![
                ToolchainCheck { name: "cargo".to_string(),   command: "cargo --version".to_string(),         required: true  },
                ToolchainCheck { name: "rustc".to_string(),   command: "rustc --version".to_string(),          required: true  },
                ToolchainCheck { name: "rustfmt".to_string(), command: "cargo fmt --version".to_string(),      required: false },
                ToolchainCheck { name: "clippy".to_string(),  command: "cargo clippy --version".to_string(),   required: false },
            ],
            filesystem_contract: FilesystemContract {
                required_actions: vec![
                    "write".to_string(), "read".to_string(), "list".to_string(),
                    "mkdir".to_string(), "zip_dir".to_string(),
                ],
                auto_create_parent_dirs: true,
            },
            verification_recipes: recipes,
            verification_levels: VerificationLevels {
                minimal:  vec!["check_project".to_string()],
                standard: vec!["check_project".to_string(), "build_release".to_string()],
                strict:   vec![
                    "format_check".to_string(), "lint".to_string(),
                    "test".to_string(), "build_release".to_string(),
                ],
            },
            common_failures: vec![
                "Inventing crate names that don't exist on crates.io".to_string(),
                "Missing feature flags in Cargo.toml (e.g. tokio/full, serde/derive)".to_string(),
                "Including target/ in zip — always exclude it".to_string(),
                "Missing src/main.rs for binary crates".to_string(),
                "Wrong Cargo.toml edition — must be 2021".to_string(),
                "Using SDL2 (needs native deps) instead of macroquad for GUI games".to_string(),
            ],
            repair_strategies: vec![
                "Read Cargo.toml first before modifying — check existing deps and edition".to_string(),
                "For GUI games use macroquad (no native deps); for terminal use crossterm".to_string(),
                "Check exact crate name + version on crates.io before putting it in Cargo.toml".to_string(),
                "If build fails with unresolved import, check feature flags in Cargo.toml".to_string(),
                "If zip is large, recreate source zip excluding target/".to_string(),
            ],
            packaging: PackagingSpec {
                source_zip_exclude: vec![
                    "target/".to_string(), ".git/".to_string(), "*.zip".to_string(),
                ],
                binary_name_template: Some("{project_name}".to_string()),
            },
            system_prompt_addition: concat!(
                "RUST ADAPTER:\n",
                "- cargo check verifies compilation; cargo build --release produces the binary.\n",
                "- GUI games: use macroquad (no native deps). Terminal games: use crossterm.\n",
                "- NEVER invent crate names. Only use crates you know exist on crates.io.\n",
                "- Cargo.toml must have edition=\"2021\". src/main.rs is the binary entry point.\n",
                "- ZIP: use filesystem zip_dir action with exclude:[\"target/\",\".git/\"].\n",
                "  Params: {\"action\":\"zip_dir\",\"source_dir\":\"<dir>\",\"output_path\":\"<name>.zip\",\"exclude\":[\"target/\",\".git/\"]}\n",
            ).to_string(),
        }
    }

    fn resolve_profile(&self, framework: Option<&str>, project_type: Option<&str>) -> String {
        let f = framework.unwrap_or("").to_lowercase();
        let p = project_type.unwrap_or("").to_lowercase();
        if f.contains("macroquad") { return "gui-macroquad".to_string(); }
        if f.contains("crossterm") || p.contains("terminal") || p.contains("tui") {
            return "terminal-game-crossterm".to_string();
        }
        if f.contains("axum") || p.contains("web") || p.contains("api") {
            return "web-axum".to_string();
        }
        if f.contains("actix") { return "web-actix".to_string(); }
        if p.contains("library") || p.contains("lib") { return "library".to_string(); }
        "cli".to_string()
    }
}
