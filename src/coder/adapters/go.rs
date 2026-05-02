use std::collections::HashMap;
use crate::coder::adapter::{
    AdapterManifest, FilesystemContract, LanguageAdapter, PackagingSpec, ToolchainCheck,
    VerificationLevels,
};

pub struct GoAdapter;

impl LanguageAdapter for GoAdapter {
    fn manifest(&self) -> AdapterManifest {
        let mut recipes = HashMap::new();
        recipes.insert("check_project".to_string(), "go build ./...".to_string());
        recipes.insert("test".to_string(), "go test ./...".to_string());
        recipes.insert("format_check".to_string(), "gofmt -l .".to_string());
        recipes.insert("lint".to_string(), "go vet ./...".to_string());
        recipes.insert("build_release".to_string(), "go build -o bin/ ./...".to_string());

        AdapterManifest {
            language: "go".to_string(),
            aliases: vec!["golang".to_string()],
            project_markers: vec![
                "go.mod".to_string(), "go.sum".to_string(), "main.go".to_string(),
            ],
            profiles: vec!["cli".to_string(), "web-api".to_string(), "library".to_string()],
            toolchain_checks: vec![
                ToolchainCheck { name: "go".to_string(), command: "go version".to_string(), required: true },
            ],
            filesystem_contract: FilesystemContract {
                required_actions: vec!["write".to_string(), "read".to_string(), "zip_dir".to_string()],
                auto_create_parent_dirs: true,
            },
            verification_recipes: recipes,
            verification_levels: VerificationLevels {
                minimal:  vec!["check_project".to_string()],
                standard: vec!["check_project".to_string(), "test".to_string()],
                strict:   vec!["format_check".to_string(), "lint".to_string(), "test".to_string(), "build_release".to_string()],
            },
            common_failures: vec![
                "Missing go.mod file — required for all Go modules".to_string(),
                "Package name mismatch between directory and package declaration".to_string(),
                "Missing main package / main() function for binary".to_string(),
            ],
            repair_strategies: vec![
                "Always create go.mod with correct module name and go version".to_string(),
                "Package must match directory name; main package for executables".to_string(),
            ],
            packaging: PackagingSpec {
                source_zip_exclude: vec![
                    ".git/".to_string(), "bin/".to_string(),
                ],
                binary_name_template: Some("{project_name}".to_string()),
            },
            system_prompt_addition: concat!(
                "GO ADAPTER:\n",
                "- go.mod is required. Use 'go build ./...' to verify.\n",
                "- ZIP excludes: .git/, bin/.\n",
            ).to_string(),
        }
    }

    fn resolve_profile(&self, _framework: Option<&str>, project_type: Option<&str>) -> String {
        let p = project_type.unwrap_or("").to_lowercase();
        if p.contains("web") || p.contains("api") { return "web-api".to_string(); }
        if p.contains("library") { return "library".to_string(); }
        "cli".to_string()
    }
}
