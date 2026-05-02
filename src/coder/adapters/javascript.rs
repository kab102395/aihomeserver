use std::collections::HashMap;
use crate::coder::adapter::{
    AdapterManifest, FilesystemContract, LanguageAdapter, PackagingSpec, ToolchainCheck,
    VerificationLevels,
};

pub struct JavaScriptAdapter;

impl LanguageAdapter for JavaScriptAdapter {
    fn manifest(&self) -> AdapterManifest {
        let mut recipes = HashMap::new();
        recipes.insert("install_deps".to_string(), "npm install".to_string());
        recipes.insert("check_project".to_string(), "npm run typecheck 2>/dev/null || node --check index.js 2>/dev/null || true".to_string());
        recipes.insert("test".to_string(), "npm test".to_string());
        recipes.insert("build_release".to_string(), "npm run build".to_string());

        AdapterManifest {
            language: "javascript".to_string(),
            aliases: vec!["js".to_string(), "typescript".to_string(), "ts".to_string(), "node".to_string()],
            project_markers: vec![
                "package.json".to_string(), "tsconfig.json".to_string(),
                "index.js".to_string(), "index.ts".to_string(),
            ],
            profiles: vec![
                "node-cli".to_string(), "express-api".to_string(),
                "vite-react".to_string(), "next".to_string(), "library".to_string(),
            ],
            toolchain_checks: vec![
                ToolchainCheck { name: "node".to_string(), command: "node --version".to_string(), required: true },
                ToolchainCheck { name: "npm".to_string(),  command: "npm --version".to_string(),  required: true },
            ],
            filesystem_contract: FilesystemContract {
                required_actions: vec!["write".to_string(), "read".to_string(), "zip_dir".to_string()],
                auto_create_parent_dirs: true,
            },
            verification_recipes: recipes,
            verification_levels: VerificationLevels {
                minimal:  vec!["check_project".to_string()],
                standard: vec!["install_deps".to_string(), "check_project".to_string()],
                strict:   vec!["install_deps".to_string(), "check_project".to_string(), "test".to_string()],
            },
            common_failures: vec![
                "Missing package.json when using npm modules".to_string(),
                "ESM vs CommonJS mismatch (require vs import)".to_string(),
                "Missing npm install before running".to_string(),
            ],
            repair_strategies: vec![
                "Always include package.json with correct main entry and scripts".to_string(),
                "Match module type: use 'type': 'module' in package.json for ESM".to_string(),
            ],
            packaging: PackagingSpec {
                source_zip_exclude: vec![
                    "node_modules/".to_string(), ".git/".to_string(),
                    "dist/".to_string(), ".next/".to_string(),
                ],
                binary_name_template: None,
            },
            system_prompt_addition: concat!(
                "JAVASCRIPT/TYPESCRIPT ADAPTER:\n",
                "- Always include package.json with name, version, main, and scripts.\n",
                "- ZIP excludes: node_modules/, dist/, .git/.\n",
                "- Run npm install before any npm scripts.\n",
            ).to_string(),
        }
    }

    fn resolve_profile(&self, framework: Option<&str>, _project_type: Option<&str>) -> String {
        let f = framework.unwrap_or("").to_lowercase();
        if f.contains("react") || f.contains("vite") { return "vite-react".to_string(); }
        if f.contains("next") { return "next".to_string(); }
        if f.contains("express") { return "express-api".to_string(); }
        "node-cli".to_string()
    }
}
