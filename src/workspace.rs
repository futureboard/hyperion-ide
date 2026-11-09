use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

#[derive(Clone, Debug)]
pub struct FileTree {
    root: Option<PathBuf>,
    expanded: std::collections::HashSet<PathBuf>,
    pub entries: Vec<TreeEntry>,
    pub error: Option<String>,
}

impl Default for FileTree {
    fn default() -> Self {
        Self::empty()
    }
}

impl FileTree {
    pub fn empty() -> Self {
        Self {
            root: None,
            expanded: std::collections::HashSet::new(),
            entries: Vec::new(),
            error: None,
        }
    }

    pub fn open(root: PathBuf) -> Self {
        let mut tree = Self::empty();
        tree.set_root(root);
        tree
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.expanded.clear();
        self.root = Some(root);
        self.rebuild();
    }

    pub fn refresh(&mut self) {
        self.rebuild();
    }

    pub fn toggle_dir(&mut self, path: &Path) {
        if !self.expanded.insert(path.to_path_buf()) {
            self.expanded.remove(path);
        }
        self.rebuild();
    }

    pub fn expand_to(&mut self, path: &Path) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let Ok(relative) = path.strip_prefix(&root) else {
            return;
        };
        let mut acc = root;
        let mut components = relative.components().peekable();
        let mut changed = false;
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                break;
            }
            acc.push(component);
            changed |= self.expanded.insert(acc.clone());
        }
        if changed {
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        self.entries.clear();
        let Some(root) = self.root.clone() else {
            self.error = None;
            return;
        };
        if !root.is_dir() {
            self.error = Some(format!("{} is not a folder", root.display()));
            return;
        }
        self.error = None;
        let mut stack = Vec::new();
        if let Ok(canon) = root.canonicalize() {
            stack.push(canon);
        }
        if let Err(error) = self.walk(&root, 0, &mut stack) {
            self.error = Some(error);
        }
    }

    fn walk(
        &mut self,
        dir: &Path,
        depth: usize,
        ancestors: &mut Vec<PathBuf>,
    ) -> Result<(), String> {
        if depth > 64 {
            return Ok(());
        }
        let mut listed = Vec::new();
        let read = fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        for entry in read {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = entry.file_type().ok();
            let is_dir = match kind {
                Some(kind) if kind.is_dir() => true,
                Some(kind) if kind.is_symlink() => path.is_dir(),
                _ => false,
            };
            listed.push((path, name, is_dir));
        }
        listed.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
        });

        for (path, name, is_dir) in listed {
            let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
            let cycle = is_dir && ancestors.iter().any(|ancestor| ancestor == &canon);
            let expanded = is_dir && self.expanded.contains(&path) && !cycle;
            self.entries.push(TreeEntry {
                path: path.clone(),
                name,
                depth,
                is_dir,
                expanded,
            });
            if expanded {
                ancestors.push(canon);
                self.walk(&path, depth + 1, ancestors)?;
                ancestors.pop();
            }
        }
        Ok(())
    }
}

pub fn file_title(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub fn display_path(root: Option<&Path>, path: &Path) -> String {
    if let Some(root) = root
        && let Ok(relative) = path.strip_prefix(root)
        && !relative.as_os_str().is_empty()
    {
        return relative.display().to_string();
    }
    path.display().to_string()
}

pub fn language_name(path: Option<&Path>) -> &'static str {
    let Some(path) = path else {
        return "Plain Text";
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if name.eq_ignore_ascii_case("cargo.toml") {
        return "TOML";
    }
    if name.eq_ignore_ascii_case("makefile") {
        return "Makefile";
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("rs") => "Rust",
        Some("toml") => "TOML",
        Some("md" | "markdown") => "Markdown",
        Some("json") => "JSON",
        Some("js" | "mjs" | "cjs") => "JavaScript",
        Some("ts" | "tsx") => "TypeScript",
        Some("py") => "Python",
        Some("go") => "Go",
        Some("c" | "h") => "C",
        Some("cc" | "cpp" | "cxx" | "hpp" | "hh") => "C++",
        Some("java") => "Java",
        Some("html" | "htm") => "HTML",
        Some("css") => "CSS",
        Some("sh" | "bash" | "zsh") => "Shell",
        Some("yml" | "yaml") => "YAML",
        Some("xml") => "XML",
        Some("sql") => "SQL",
        Some("lua") => "Lua",
        Some("rb") => "Ruby",
        Some("swift") => "Swift",
        Some("kt" | "kts") => "Kotlin",
        Some("zig") => "Zig",
        _ => "Plain Text",
    }
}

pub fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_and_display_path() {
        assert_eq!(language_name(Some(Path::new("src/main.rs"))), "Rust");
        assert_eq!(language_name(Some(Path::new("Cargo.toml"))), "TOML");
        assert_eq!(language_name(None), "Plain Text");
        let root = Path::new("/work/app");
        assert_eq!(
            display_path(Some(root), Path::new("/work/app/src/main.rs")),
            "src/main.rs"
        );
        assert_eq!(
            display_path(Some(root), Path::new("/other/file.txt")),
            "/other/file.txt"
        );
    }

    #[test]
    fn tree_lists_children_and_expands_one_level() {
        let root = std::env::temp_dir().join(format!("hyperion-ide-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.txt"), "hi").unwrap();
        fs::write(root.join("sub").join("b.txt"), "yo").unwrap();

        let mut tree = FileTree::open(root.clone());
        assert!(tree.error.is_none());
        let names: Vec<_> = tree
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, vec!["sub", "a.txt"]);
        assert!(tree.entries.iter().all(|entry| entry.depth == 0));

        let sub = tree
            .entries
            .iter()
            .find(|entry| entry.name == "sub")
            .unwrap()
            .path
            .clone();
        tree.toggle_dir(&sub);
        let nested = tree
            .entries
            .iter()
            .find(|entry| entry.name == "b.txt")
            .unwrap();
        assert_eq!(nested.depth, 1);
        assert!(nested.path.ends_with("b.txt"));

        let _ = fs::remove_dir_all(&root);
    }
}
