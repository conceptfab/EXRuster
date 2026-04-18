use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FolderNode {
    pub path: PathBuf,
    pub display_name: String,
    pub depth: i32,
    pub has_children: bool,
}

/// List immediate subdirectories of `dir` (no recursion).
/// Skips hidden entries (`.` prefix) and unreadable entries.
pub fn list_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new(); };
    let mut out: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|s| !s.starts_with('.'))
                .unwrap_or(false)
        })
        .collect();
    out.sort_by(|a, b| {
        a.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase()
            .cmp(
                &b.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase(),
            )
    });
    out
}

fn display_name(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| p.display().to_string())
}

/// Build a flat, depth-ordered tree from `root`, expanding only those paths
/// present in `expanded`. Each node carries its depth and a `has_children`
/// flag (cheap filesystem check).
pub fn build_flat_tree(
    root: &Path,
    expanded: &std::collections::HashSet<PathBuf>,
) -> Vec<FolderNode> {
    let mut out = Vec::new();
    fn walk(
        dir: &Path,
        depth: i32,
        expanded: &std::collections::HashSet<PathBuf>,
        out: &mut Vec<FolderNode>,
    ) {
        let subs = list_subdirs(dir);
        let has_children = !subs.is_empty();
        out.push(FolderNode {
            path: dir.to_path_buf(),
            display_name: display_name(dir),
            depth,
            has_children,
        });
        if expanded.contains(dir) {
            for s in subs {
                walk(&s, depth + 1, expanded, out);
            }
        }
    }
    walk(root, 0, expanded, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_subdirs_alphabetically() {
        let tmp = std::env::temp_dir().join(format!("exruster-ft-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("bravo")).unwrap();
        std::fs::create_dir_all(tmp.join("alpha")).unwrap();
        let subs = list_subdirs(&tmp);
        assert_eq!(
            subs.iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            vec!["alpha".to_string(), "bravo".to_string()]
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
