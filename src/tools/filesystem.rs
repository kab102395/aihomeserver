use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Instant;

use super::Tool;
use crate::state::{ErrorType, ToolResult};
use crate::worker::{
    WorkerClient, WorkerFilesystemDeleteRequest, WorkerFilesystemFindRequest,
    WorkerFilesystemGrepRequest, WorkerFilesystemListRequest, WorkerFilesystemMkdirRequest,
    WorkerFilesystemReadRequest, WorkerFilesystemRenameRequest, WorkerFilesystemWriteRequest,
};

/// Tool for safe workspace file operations (read/write/list/find/grep/etc.).
///
/// Connection:
/// - The executor prefers using this tool for repo inspection and file edits because it’s
///   portable (no shell differences) and can enforce path traversal protections.
pub struct FilesystemTool {
    base_dir: PathBuf,
    worker: Option<WorkerClient>,
    execution_mode: String,
}

impl FilesystemTool {
    /// Create a filesystem tool rooted at `base_dir` (created if missing).
    pub fn new(
        base_dir: impl Into<PathBuf>,
        worker: Option<WorkerClient>,
        execution_mode: impl Into<String>,
    ) -> std::io::Result<Self> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir)?;
        Ok(Self {
            base_dir,
            worker,
            execution_mode: execution_mode.into(),
        })
    }

    /// Resolve a user-supplied relative path against `base_dir` safely.
    ///
    /// Why this exists:
    /// - Prevents `..` path traversal and absolute-path escapes.
    /// - Keeps tool operations constrained to the configured workspace root.
    fn resolve(&self, path: &str) -> PathBuf {
        // Prevent path traversal: strip leading / and ..
        let clean: PathBuf = path
            .split(['/', '\\'])
            .filter(|c| !c.is_empty() && *c != "..")
            .collect();
        self.base_dir.join(clean)
    }

    /// Compute a small stable hash of bytes for change detection/checkpointing.
    ///
    /// Connection:
    /// - Used in tool outputs to let the runtime/UI detect whether content changed.
    fn hash_bytes(data: &[u8]) -> String {
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Convert an absolute path back into a workspace-relative path for UI display.
    fn rel(&self, full: &PathBuf) -> String {
        full.strip_prefix(&self.base_dir)
            .unwrap_or(full)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// Best-effort binary detection to avoid returning unreadable blobs as “text”.
    fn is_probably_text(bytes: &[u8]) -> bool {
        // NUL byte is a strong signal for binary.
        !bytes.iter().any(|b| *b == 0)
    }

    /// Walk files under `root` up to bounds, skipping symlinks.
    ///
    /// Why this exists:
    /// - `find`/`grep` need a file list, but we must cap work to avoid huge traversals.
    /// - Skipping symlinks prevents cycles and accidental escapes.
    fn walk_files(&self, root: &PathBuf, max_depth: usize, max_files: usize) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack: Vec<(PathBuf, usize)> = vec![(root.clone(), 0)];

        while let Some((dir, depth)) = stack.pop() {
            if depth > max_depth {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if out.len() >= max_files {
                    return out;
                }
                let path = entry.path();
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.file_type().is_symlink() {
                    continue;
                }
                if meta.is_dir() {
                    stack.push((path, depth + 1));
                } else if meta.is_file() {
                    out.push(path);
                }
            }
        }

        out
    }

    fn remote_enabled(&self) -> bool {
        (self.execution_mode.trim().eq_ignore_ascii_case("remote")
            || self.execution_mode.trim().eq_ignore_ascii_case("auto"))
            && self.worker.as_ref().map(|w| w.is_enabled()).unwrap_or(false)
    }
}

#[async_trait]
impl Tool for FilesystemTool {
    /// Canonical tool name used in planner/executor `tool_binding`.
    fn name(&self) -> &str {
        "filesystem"
    }

    /// Execute a filesystem action (read/write/list/find/grep/etc.).
    async fn execute(&self, params: Value) -> ToolResult {
        let action_raw = params["action"].as_str().unwrap_or("").to_string();
        // Compatibility: tolerate alternate action names emitted by different planners/models.
        let action = match action_raw.as_str() {
            // write synonyms
            "write_file" | "create_file" | "create" | "touch" => "write".to_string(),
            // read synonyms
            "read_file" => "read".to_string(),
            // list synonyms
            "ls" => "list".to_string(),
            // mkdir synonyms
            "mkdir" | "create_dir" | "create_directory" | "makedir" => "mkdir".to_string(),
            other => other.to_string(),
        };
        // zip_dir can specify source_dir instead of path; allow empty path for it
        let path = match params["path"].as_str() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => {
                // zip_dir uses source_dir; other actions truly need path
                if action != "zip_dir" && action != "zip" {
                    return ToolResult::err(ErrorType::Tool, "missing_path", "params.path is required");
                }
                params.get("source_dir")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".")
                    .to_string()
            }
        };

        if self.remote_enabled() {
            if let Some(worker) = &self.worker {
                let remote = match action.as_str() {
                    "write" => {
                        let content = params["content"].as_str().unwrap_or("").to_string();
                        worker
                            .filesystem_write(&WorkerFilesystemWriteRequest {
                                path: path.clone(),
                                content: Some(content),
                                contents_b64: None,
                            })
                            .await
                    }
                    "read" => {
                        worker
                            .filesystem_read(&WorkerFilesystemReadRequest { path: path.clone() })
                            .await
                    }
                    "list" => {
                        let depth = params
                            .get("depth")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as u8);
                        worker
                            .filesystem_list(&WorkerFilesystemListRequest {
                                path: path.clone(),
                                depth,
                            })
                            .await
                    }
                    "find" => {
                        let pattern = match params.get("pattern").and_then(|v| v.as_str()) {
                            Some(p) if !p.trim().is_empty() => p.to_string(),
                            _ => {
                                return ToolResult::err(
                                    ErrorType::Tool,
                                    "missing_param",
                                    "params.pattern is required",
                                )
                            }
                        };
                        worker
                            .filesystem_find(&WorkerFilesystemFindRequest {
                                path: path.clone(),
                                pattern,
                                max_depth: params
                                    .get("max_depth")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v as usize),
                                max_files: params
                                    .get("max_files")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v as usize),
                                max_results: params
                                    .get("max_results")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v as usize),
                            })
                            .await
                    }
                    "grep" => {
                        let query = match params.get("query").and_then(|v| v.as_str()) {
                            Some(q) if !q.trim().is_empty() => q.to_string(),
                            _ => {
                                return ToolResult::err(
                                    ErrorType::Tool,
                                    "missing_param",
                                    "params.query is required",
                                )
                            }
                        };
                        worker
                            .filesystem_grep(&WorkerFilesystemGrepRequest {
                                path: path.clone(),
                                query,
                                max_depth: params
                                    .get("max_depth")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v as usize),
                                max_files: params
                                    .get("max_files")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v as usize),
                                max_results: params
                                    .get("max_results")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v as usize),
                                max_bytes_per_file: params
                                    .get("max_bytes_per_file")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v as usize),
                            })
                            .await
                    }
                    "delete" => {
                        worker
                            .filesystem_delete(&WorkerFilesystemDeleteRequest {
                                path: path.clone(),
                            })
                            .await
                    }
                    "mkdir" => {
                        worker
                            .filesystem_mkdir(&WorkerFilesystemMkdirRequest {
                                path: path.clone(),
                            })
                            .await
                    }
                    "rename" => {
                        let to = match params.get("to").and_then(|v| v.as_str()) {
                            Some(to) if !to.trim().is_empty() => to.to_string(),
                            _ => {
                                return ToolResult::err(
                                    ErrorType::Tool,
                                    "missing_param",
                                    "params.to is required",
                                )
                            }
                        };
                        worker
                            .filesystem_rename(&WorkerFilesystemRenameRequest {
                                from: path.clone(),
                                to,
                            })
                            .await
                    }
                    "zip_dir" | "zip" => {
                        return ToolResult::err(
                            ErrorType::Tool,
                            "unsupported_remote_action",
                            "zip_dir is not yet supported against the VM workspace",
                        )
                    }
                    _ => {
                        return ToolResult::err(
                            ErrorType::Tool,
                            "unsupported_action",
                            &action,
                        )
                    }
                };

                return match remote {
                    Ok(result) => result,
                    Err(e) => ToolResult::err(
                        ErrorType::Env,
                        "worker_filesystem_failed",
                        &e.to_string(),
                    ),
                };
            }
        }

        match action.as_str() {
            "write" => {
                let content = params["content"].as_str().unwrap_or("");
                let overwrite = params["overwrite"].as_bool().unwrap_or(true);
                let full = self.resolve(&path);

                if let Some(parent) = full.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return ToolResult::err(ErrorType::Env, "mkdir_failed", &e.to_string());
                    }
                }
                if full.exists() && !overwrite {
                    return ToolResult::err(
                        ErrorType::Permission,
                        "file_exists_no_overwrite",
                        &full.display().to_string(),
                    );
                }
                if let Err(e) = std::fs::write(&full, content) {
                    return ToolResult::err(ErrorType::Env, "write_failed", &e.to_string());
                }

                let hash = Self::hash_bytes(content.as_bytes());
                let cp = json!({
                    "type": "filesystem_write",
                    "path": full.display().to_string(),
                    "hash": hash,
                    "bytes": content.len(),
                });
                ToolResult::ok(
                    json!({
                        "path": full.display().to_string(),
                        "bytes_written": content.len(),
                        "hash": hash,
                    }),
                    Some(cp),
                )
            }

            "mkdir" => {
                let full = self.resolve(&path);
                match std::fs::create_dir_all(&full) {
                    Ok(_) => ToolResult::ok(
                        json!({
                            "path": full.display().to_string(),
                            "created": true
                        }),
                        Some(json!({
                            "type": "filesystem_mkdir",
                            "path": full.display().to_string()
                        })),
                    ),
                    Err(e) => ToolResult::err(ErrorType::Env, "mkdir_failed", &e.to_string()),
                }
            }

            "read" => {
                let full = self.resolve(&path);
                match std::fs::read(&full) {
                    Ok(bytes) => {
                        let content = String::from_utf8_lossy(&bytes).to_string();
                        let hash = Self::hash_bytes(&bytes);
                        ToolResult::ok(
                            json!({
                                "path": full.display().to_string(),
                                "content": content,
                                "hash": hash,
                            }),
                            Some(json!({
                                "type": "filesystem_read",
                                "path": full.display().to_string(),
                                "hash": hash,
                            })),
                        )
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => ToolResult::err(
                        ErrorType::Tool,
                        "file_not_found",
                        &full.display().to_string(),
                    ),
                    Err(e) => ToolResult::err(ErrorType::Env, "read_failed", &e.to_string()),
                }
            }

            "list" => {
                let full = self.resolve(&path);
                match std::fs::read_dir(&full) {
                    Ok(entries) => {
                        let mut files: Vec<String> = entries
                            .filter_map(|e| e.ok())
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .collect();
                        files.sort();
                        ToolResult::ok(
                            json!({ "path": full.display().to_string(), "entries": files }),
                            None,
                        )
                    }
                    Err(e) => ToolResult::err(ErrorType::Env, "list_failed", &e.to_string()),
                }
            }

            // Find files by name (substring match) under a directory tree.
            // Params:
            // - path (required): directory to search within (relative to workspace root)
            // - pattern (required): substring to match (case-insensitive)
            // - max_depth (optional, default 6)
            // - max_files (optional, default 4000)
            // - max_results (optional, default 200)
            "find" => {
                let pattern = match params.get("pattern").and_then(|v| v.as_str()) {
                    Some(p) if !p.trim().is_empty() => p.to_lowercase(),
                    _ => {
                        return ToolResult::err(
                            ErrorType::Tool,
                            "missing_param",
                            "params.pattern is required",
                        )
                    }
                };
                let max_depth = params
                    .get("max_depth")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(6) as usize;
                let max_files = params
                    .get("max_files")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(4000) as usize;
                let max_results = params
                    .get("max_results")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(200) as usize;

                let root = self.resolve(&path);
                if !root.is_dir() {
                    return ToolResult::err(
                        ErrorType::Tool,
                        "not_a_directory",
                        &root.display().to_string(),
                    );
                }

                let start = Instant::now();
                let files = self.walk_files(&root, max_depth, max_files);
                let mut matches: Vec<String> = Vec::new();
                for f in files.iter() {
                    let rel = self.rel(f);
                    if rel.to_lowercase().contains(&pattern) {
                        matches.push(rel);
                        if matches.len() >= max_results {
                            break;
                        }
                    }
                }
                matches.sort();

                ToolResult::ok(
                    json!({
                        "root": self.rel(&root),
                        "pattern": pattern,
                        "matches": matches,
                        "files_scanned": files.len(),
                        "max_depth": max_depth,
                        "max_files": max_files,
                        "max_results": max_results,
                        "duration_ms": start.elapsed().as_millis(),
                    }),
                    None,
                )
            }

            // Search inside files for a substring (case-insensitive) under a directory tree.
            // This is a safe, dependency-free alternative to relying on `rg` in the shell tool.
            // Params:
            // - path (required): directory to search within
            // - query (required): substring to search (case-insensitive)
            // - max_depth (optional, default 6)
            // - max_files (optional, default 1500)
            // - max_results (optional, default 200)
            // - max_bytes_per_file (optional, default 200000)
            "grep" => {
                let query_raw = match params.get("query").and_then(|v| v.as_str()) {
                    Some(q) if !q.trim().is_empty() => q.to_string(),
                    _ => {
                        return ToolResult::err(
                            ErrorType::Tool,
                            "missing_param",
                            "params.query is required",
                        )
                    }
                };
                let query = query_raw.to_lowercase();
                let max_depth = params
                    .get("max_depth")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(6) as usize;
                let max_files = params
                    .get("max_files")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1500) as usize;
                let max_results = params
                    .get("max_results")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(200) as usize;
                let max_bytes_per_file = params
                    .get("max_bytes_per_file")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(200_000) as usize;

                let root = self.resolve(&path);
                if !root.is_dir() {
                    return ToolResult::err(
                        ErrorType::Tool,
                        "not_a_directory",
                        &root.display().to_string(),
                    );
                }

                let start = Instant::now();
                let files = self.walk_files(&root, max_depth, max_files);

                let mut matches: Vec<Value> = Vec::new();
                let mut files_scanned = 0usize;
                let mut files_skipped = 0usize;

                for f in files.iter() {
                    if matches.len() >= max_results {
                        break;
                    }
                    let Ok(bytes) = std::fs::read(f) else {
                        files_skipped += 1;
                        continue;
                    };
                    files_scanned += 1;

                    if bytes.len() > max_bytes_per_file {
                        files_skipped += 1;
                        continue;
                    }
                    if !Self::is_probably_text(&bytes) {
                        files_skipped += 1;
                        continue;
                    }

                    let text = String::from_utf8_lossy(&bytes);
                    for (idx, line) in text.lines().enumerate() {
                        if matches.len() >= max_results {
                            break;
                        }
                        if line.to_lowercase().contains(&query) {
                            matches.push(json!({
                                "path": self.rel(f),
                                "line": idx + 1,
                                "text": line.trim(),
                            }));
                        }
                    }
                }

                ToolResult::ok(
                    json!({
                        "root": self.rel(&root),
                        "query": query_raw,
                        "matches": matches,
                        "files_scanned": files_scanned,
                        "files_skipped": files_skipped,
                        "max_depth": max_depth,
                        "max_files": max_files,
                        "max_results": max_results,
                        "max_bytes_per_file": max_bytes_per_file,
                        "duration_ms": start.elapsed().as_millis(),
                    }),
                    None,
                )
            }

            // Create a ZIP archive of a directory inside the workspace.
            // Params:
            // - source_dir (required): directory to zip (relative to workspace root)
            // - output_path (required): where to write the .zip (relative to workspace root)
            // - exclude (optional): list of glob-style prefixes to skip (default: ["target/", ".git/"])
            //
            // Returns: { path, entries: [String], size_bytes }
            // Errors if source_dir does not exist or zip cannot be written.
            "zip_dir" | "zip" => {
                // resolve source_dir
                let source_rel = match params.get("source_dir").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => path.clone(), // fall back to "path" param
                };
                let source = self.resolve(&source_rel);
                if !source.is_dir() {
                    return ToolResult::err(
                        ErrorType::Tool,
                        "source_not_found",
                        &format!("source_dir '{}' does not exist or is not a directory", source_rel),
                    );
                }

                // resolve output_path
                let output_rel = match params.get("output_path").and_then(|v| v.as_str()) {
                    Some(p) if !p.is_empty() => p.to_string(),
                    _ => format!("{}.zip", source_rel.trim_end_matches('/')),
                };
                let output = self.resolve(&output_rel);
                if let Some(parent) = output.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return ToolResult::err(ErrorType::Env, "mkdir_failed", &e.to_string());
                    }
                }

                // build exclude prefix list (relative to source_dir)
                let default_excludes = vec!["target/".to_string(), ".git/".to_string()];
                let excludes: Vec<String> = params
                    .get("exclude")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| s.trim_start_matches("./").to_string())
                            .collect()
                    })
                    .unwrap_or(default_excludes);

                // collect all files under source
                let all_files = self.walk_files(&source, 20, 50_000);

                // create the zip
                let zip_file = match std::fs::File::create(&output) {
                    Ok(f) => f,
                    Err(e) => return ToolResult::err(ErrorType::Env, "zip_create_failed", &e.to_string()),
                };
                let mut zip_writer = zip::ZipWriter::new(zip_file);
                let zip_opts = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(0o644);

                let mut entries: Vec<String> = Vec::new();

                for file_path in &all_files {
                    // compute path relative to source_dir
                    let rel = match file_path.strip_prefix(&source) {
                        Ok(r) => r.to_string_lossy().replace('\\', "/"),
                        Err(_) => continue,
                    };

                    // apply excludes
                    let excluded = excludes.iter().any(|ex| {
                        let ex_norm = ex.trim_end_matches('/');
                        rel == ex_norm
                            || rel.starts_with(&format!("{}/", ex_norm))
                            || rel.starts_with(ex.as_str())
                    });
                    if excluded {
                        continue;
                    }

                    let Ok(bytes) = std::fs::read(file_path) else { continue };

                    // entry name inside the zip is source_dir/relative
                    let entry_name = format!("{}/{}", source_rel.trim_end_matches('/'), rel);
                    if zip_writer.start_file(&entry_name, zip_opts).is_err() {
                        continue;
                    }
                    use std::io::Write;
                    if zip_writer.write_all(&bytes).is_err() {
                        continue;
                    }
                    entries.push(entry_name);
                }

                if let Err(e) = zip_writer.finish() {
                    return ToolResult::err(ErrorType::Env, "zip_finish_failed", &e.to_string());
                }

                let size_bytes = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
                if size_bytes == 0 {
                    return ToolResult::err(
                        ErrorType::Tool,
                        "zip_empty",
                        "zip file was created but has size 0",
                    );
                }

                let out_rel = self.rel(&output);
                ToolResult::ok(
                    json!({
                        "path": out_rel,
                        "entries": entries,
                        "entry_count": entries.len(),
                        "size_bytes": size_bytes,
                    }),
                    Some(json!({
                        "type": "filesystem_zip",
                        "path": out_rel,
                        "entry_count": entries.len(),
                        "size_bytes": size_bytes,
                    })),
                )
            }

            "delete" => {
                let full = self.resolve(&path);
                if !full.exists() {
                    return ToolResult::err(
                        ErrorType::Tool,
                        "file_not_found",
                        &full.display().to_string(),
                    );
                }
                let result = if full.is_dir() {
                    std::fs::remove_dir_all(&full)
                } else {
                    std::fs::remove_file(&full)
                };
                match result {
                    Ok(_) => ToolResult::ok(
                        json!({ "deleted": full.display().to_string() }),
                        Some(
                            json!({ "type": "filesystem_delete", "path": full.display().to_string() }),
                        ),
                    ),
                    Err(e) => ToolResult::err(ErrorType::Env, "delete_failed", &e.to_string()),
                }
            }

            _ => ToolResult::err(ErrorType::Tool, "unsupported_action", &action),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_base() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("aihomeserver_fs_test_{}", uuid::Uuid::new_v4()));
        p
    }

    #[tokio::test]
    async fn find_finds_files_by_substring() {
        let base = tmp_base();
        let tool = FilesystemTool::new(&base, None, "local").expect("tmp dir created");
        let _ = tool
            .execute(json!({
                "action": "write",
                "path": "src/main.rs",
                "content": "fn main() { println!(\"hello\"); }\n"
            }))
            .await;

        let r = tool
            .execute(json!({
                "action": "find",
                "path": ".",
                "pattern": "main.rs"
            }))
            .await;
        assert!(r.success, "find should succeed");
        let matches = r
            .output
            .unwrap()
            .get("matches")
            .cloned()
            .unwrap_or(Value::Null);
        let arr = matches.as_array().cloned().unwrap_or_default();
        assert!(
            arr.iter()
                .any(|v| v.as_str().unwrap_or("").ends_with("src/main.rs")),
            "expected src/main.rs in matches: {arr:?}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn grep_finds_text_in_files() {
        let base = tmp_base();
        let tool = FilesystemTool::new(&base, None, "local").expect("tmp dir created");
        let _ = tool
            .execute(json!({
                "action": "write",
                "path": "notes.txt",
                "content": "alpha\nneedle here\nomega\n"
            }))
            .await;

        let r = tool
            .execute(json!({
                "action": "grep",
                "path": ".",
                "query": "needle"
            }))
            .await;
        assert!(r.success, "grep should succeed");
        let out = r.output.unwrap();
        let hits = out
            .get("matches")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            hits.iter()
                .any(|h| h.get("path").and_then(|p| p.as_str()) == Some("notes.txt")),
            "expected notes.txt hit: {hits:?}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
