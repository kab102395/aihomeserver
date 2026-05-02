use std::collections::HashMap;
use crate::coder::adapter::{
    AdapterManifest, FilesystemContract, LanguageAdapter, PackagingSpec, ToolchainCheck,
    VerificationLevels,
};

pub struct PythonAdapter;

impl LanguageAdapter for PythonAdapter {
    fn manifest(&self) -> AdapterManifest {
        let mut recipes = HashMap::new();
        recipes.insert("check_project".to_string(), "python -m py_compile $(find . -name '*.py' | head -20 | tr '\\n' ' ')".to_string());
        recipes.insert("test".to_string(), "python -m pytest".to_string());
        recipes.insert("lint".to_string(), "ruff check .".to_string());
        recipes.insert("format_check".to_string(), "black --check .".to_string());
        recipes.insert("install_deps".to_string(), "pip install -r requirements.txt".to_string());

        AdapterManifest {
            language: "python".to_string(),
            aliases: vec!["py".to_string()],
            project_markers: vec![
                "pyproject.toml".to_string(), "requirements.txt".to_string(),
                "setup.py".to_string(), "main.py".to_string(),
            ],
            profiles: vec![
                "script".to_string(), "cli".to_string(), "fastapi".to_string(),
                "flask".to_string(), "django".to_string(), "data-science".to_string(),
            ],
            toolchain_checks: vec![
                ToolchainCheck { name: "python".to_string(), command: "python --version".to_string(), required: true },
                ToolchainCheck { name: "pip".to_string(),    command: "pip --version".to_string(),    required: false },
            ],
            filesystem_contract: FilesystemContract {
                required_actions: vec!["write".to_string(), "read".to_string(), "zip_dir".to_string()],
                auto_create_parent_dirs: true,
            },
            verification_recipes: recipes,
            verification_levels: VerificationLevels {
                minimal:  vec!["check_project".to_string()],
                standard: vec!["check_project".to_string(), "test".to_string()],
                strict:   vec!["lint".to_string(), "format_check".to_string(), "check_project".to_string(), "test".to_string()],
            },
            common_failures: vec![
                "Missing requirements.txt when using third-party libraries".to_string(),
                "Relative imports failing when run as script vs module".to_string(),
                "Missing if __name__ == '__main__' guard".to_string(),
            ],
            repair_strategies: vec![
                "Add requirements.txt with all imports that aren't stdlib".to_string(),
                "Use python -m py_compile to syntax-check without running".to_string(),
            ],
            packaging: PackagingSpec {
                source_zip_exclude: vec![
                    ".venv/".to_string(), "__pycache__/".to_string(),
                    "*.pyc".to_string(), ".git/".to_string(),
                ],
                binary_name_template: None,
            },
            system_prompt_addition: concat!(
                "PYTHON ADAPTER:\n",
                "- Include requirements.txt for any non-stdlib imports.\n",
                "- Use python -m py_compile for syntax verification.\n",
                "- ZIP excludes: .venv/, __pycache__/, *.pyc, .git/.\n",
            ).to_string(),
        }
    }

    fn resolve_profile(&self, framework: Option<&str>, _project_type: Option<&str>) -> String {
        let f = framework.unwrap_or("").to_lowercase();
        if f.contains("fastapi") { return "fastapi".to_string(); }
        if f.contains("django") { return "django".to_string(); }
        if f.contains("flask") { return "flask".to_string(); }
        "script".to_string()
    }
}
