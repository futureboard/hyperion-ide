use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceError {
    pub path: PathBuf,
    pub reason: String,
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.reason)
    }
}

impl std::error::Error for WorkspaceError {}

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
    expanded: HashSet<PathBuf>,
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
            expanded: HashSet::new(),
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

    pub fn create_file(&mut self, path: &Path) -> Result<(), WorkspaceError> {
        let (root, root_canon) = self.root_pair(path)?;
        let target = resolve_inside(&root, &root_canon, path)?;
        ensure_absent(&target)?;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| io_at(&target, error))?;
        self.refresh();
        Ok(())
    }

    pub fn create_folder(&mut self, path: &Path) -> Result<(), WorkspaceError> {
        let (root, root_canon) = self.root_pair(path)?;
        let target = resolve_inside(&root, &root_canon, path)?;
        ensure_absent(&target)?;
        fs::create_dir(&target).map_err(|error| io_at(&target, error))?;
        self.refresh();
        Ok(())
    }

    pub fn rename(&mut self, from: &Path, to: &Path) -> Result<(), WorkspaceError> {
        let (root, root_canon) = self.root_pair(from)?;
        let source = resolve_inside(&root, &root_canon, from)?;
        let dest = resolve_inside(&root, &root_canon, to)?;
        if source == dest {
            return Err(WorkspaceError {
                path: dest,
                reason: "source and destination are the same".into(),
            });
        }
        if dest.starts_with(&source) {
            return Err(WorkspaceError {
                path: dest,
                reason: "destination is inside the source".into(),
            });
        }
        fs::symlink_metadata(&source).map_err(|error| io_at(&source, error))?;
        ensure_absent(&dest)?;
        fs::rename(&source, &dest).map_err(|error| io_at(&dest, error))?;
        self.retarget_expanded(&source, &dest);
        self.refresh();
        Ok(())
    }

    pub fn delete_file(&mut self, path: &Path) -> Result<(), WorkspaceError> {
        let (root, root_canon) = self.root_pair(path)?;
        let target = resolve_inside(&root, &root_canon, path)?;
        let meta = fs::symlink_metadata(&target).map_err(|error| io_at(&target, error))?;
        if !meta.file_type().is_symlink() && meta.is_dir() {
            return Err(WorkspaceError {
                path: target,
                reason: "is a directory".into(),
            });
        }
        fs::remove_file(&target).map_err(|error| io_at(&target, error))?;
        self.forget_expanded(&target);
        self.refresh();
        Ok(())
    }

    pub fn delete_tree(&mut self, path: &Path) -> Result<(), WorkspaceError> {
        let (root, root_canon) = self.root_pair(path)?;
        let target = resolve_inside(&root, &root_canon, path)?;
        let meta = fs::symlink_metadata(&target).map_err(|error| io_at(&target, error))?;
        if meta.file_type().is_symlink() {
            return Err(refuse_symlink(&target, &root_canon));
        }
        if !meta.is_dir() {
            return Err(WorkspaceError {
                path: target,
                reason: "is not a directory".into(),
            });
        }
        let target_canon = fs::canonicalize(&target).map_err(|error| io_at(&target, error))?;
        if target_canon == root_canon {
            return Err(WorkspaceError {
                path: target,
                reason: "cannot delete the workspace root".into(),
            });
        }
        find_escaping_symlink(&root_canon, &target)?;
        remove_tree(&target)?;
        self.forget_expanded(&target);
        self.refresh();
        Ok(())
    }

    fn root_pair(&self, path: &Path) -> Result<(PathBuf, PathBuf), WorkspaceError> {
        let root = self.root.clone().ok_or_else(|| WorkspaceError {
            path: path.to_path_buf(),
            reason: "no folder is open".into(),
        })?;
        let root_canon = fs::canonicalize(&root).map_err(|error| io_at(&root, error))?;
        Ok((root, root_canon))
    }

    fn retarget_expanded(&mut self, from: &Path, to: &Path) {
        let existing: Vec<PathBuf> = self.expanded.drain().collect();
        for path in existing {
            if let Ok(suffix) = path.strip_prefix(from) {
                self.expanded.insert(to.join(suffix));
            } else {
                self.expanded.insert(path);
            }
        }
    }

    fn forget_expanded(&mut self, path: &Path) {
        self.expanded.retain(|entry| !entry.starts_with(path));
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
        let ignore = match load_ignore(&root) {
            Ok(rules) => rules,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        let root_canon = match fs::canonicalize(&root) {
            Ok(path) => path,
            Err(error) => {
                self.error = Some(format!("{}: {error}", root.display()));
                return;
            }
        };
        self.error = None;
        let mut ancestors = vec![root_canon.clone()];
        if let Err(error) = self.walk(&root, 0, &mut ancestors, &root, &root_canon, &ignore) {
            self.error = Some(error);
        }
    }

    fn walk(
        &mut self,
        dir: &Path,
        depth: usize,
        ancestors: &mut Vec<PathBuf>,
        root: &Path,
        root_canon: &Path,
        ignore: &[IgnoreRule],
    ) -> Result<(), String> {
        if depth > 64 {
            return Ok(());
        }
        let mut listed = Vec::new();
        let read = fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        for entry in read {
            let entry = entry.map_err(|error| format!("{}: {error}", dir.display()))?;
            let path = entry.path();
            if entry.file_name().as_os_str() == OsStr::new(".git") {
                continue;
            }
            let kind = entry
                .file_type()
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let looks_like_dir = kind.is_dir() || (kind.is_symlink() && path.is_dir());
            if is_ignored(ignore, root, &path, looks_like_dir) {
                continue;
            }
            let is_dir = if kind.is_symlink() {
                match symlink_target(&path, root_canon) {
                    Ok(SymlinkTarget::Inside(canon)) => {
                        canon.is_dir() && ancestors.iter().all(|ancestor| ancestor != &canon)
                    }
                    Ok(_) => false,
                    Err(error) => return Err(format!("{}: {error}", path.display())),
                }
            } else {
                kind.is_dir()
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            listed.push((path, name, is_dir));
        }
        listed.sort_by(|left, right| {
            right
                .2
                .cmp(&left.2)
                .then_with(|| left.1.to_lowercase().cmp(&right.1.to_lowercase()))
        });

        for (path, name, is_dir) in listed {
            let expanded = is_dir && self.expanded.contains(&path);
            self.entries.push(TreeEntry {
                path: path.clone(),
                name,
                depth,
                is_dir,
                expanded,
            });
            if expanded {
                let canon = path
                    .canonicalize()
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                ancestors.push(canon);
                self.walk(&path, depth + 1, ancestors, root, root_canon, ignore)?;
                ancestors.pop();
            }
        }
        Ok(())
    }
}

struct IgnoreRule {
    path: PathBuf,
    anchored: bool,
    dir_only: bool,
}

impl IgnoreRule {
    fn matches(&self, relative: &Path, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        if self.anchored {
            relative == self.path
        } else {
            relative
                .file_name()
                .is_some_and(|name| name == self.path.as_os_str())
        }
    }
}

enum SymlinkTarget {
    Inside(PathBuf),
    Outside,
    Missing(String),
    Loop(String),
}

fn is_symlink_loop(error: &std::io::Error) -> bool {
    const ELOOP_LINUX: i32 = 40;
    const ELOOP_DARWIN: i32 = 62;
    matches!(error.raw_os_error(), Some(ELOOP_LINUX | ELOOP_DARWIN))
}

fn io_at(path: &Path, error: std::io::Error) -> WorkspaceError {
    WorkspaceError {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

fn escape(path: &Path) -> WorkspaceError {
    WorkspaceError {
        path: path.to_path_buf(),
        reason: "path escapes the workspace root".into(),
    }
}

// Reject the path when ".." or an intermediate symlink resolves outside the workspace root.
fn resolve_inside(
    root: &Path,
    root_canon: &Path,
    requested: &Path,
) -> Result<PathBuf, WorkspaceError> {
    let joined = if requested.is_absolute() {
        requested.to_path_buf()
    } else if root.is_absolute() {
        root.join(requested)
    } else {
        root_canon.join(requested)
    };
    let Some(lexical) = normalize_lexical(&joined) else {
        return Err(escape(requested));
    };
    let root_norm = normalize_lexical(root).unwrap_or_else(|| root.to_path_buf());
    let inside = (!root_norm.as_os_str().is_empty() && lexical.starts_with(&root_norm))
        || lexical.starts_with(root_canon);
    if !inside {
        return Err(escape(&lexical));
    }
    ensure_no_symlink_escape(root_canon, &lexical)?;
    Ok(lexical)
}

fn normalize_lexical(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::RootDir => out.push(component.as_os_str()),
            Component::Normal(part) => out.push(part),
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
        }
    }
    Some(out)
}

fn ensure_no_symlink_escape(root_canon: &Path, lexical: &Path) -> Result<(), WorkspaceError> {
    let mut acc = PathBuf::new();
    let components: Vec<Component<'_>> = lexical.components().collect();
    for (index, component) in components.iter().enumerate() {
        acc.push(component);
        let is_last = index + 1 == components.len();
        match fs::symlink_metadata(&acc) {
            Ok(meta) if meta.file_type().is_symlink() && !is_last => {
                match symlink_target(&acc, root_canon) {
                    Ok(SymlinkTarget::Inside(_)) => {}
                    Ok(SymlinkTarget::Outside) => return Err(escape(&acc)),
                    Ok(SymlinkTarget::Missing(detail) | SymlinkTarget::Loop(detail)) => {
                        return Err(WorkspaceError {
                            path: acc,
                            reason: detail,
                        });
                    }
                    Err(error) => return Err(io_at(&acc, error)),
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_at(&acc, error)),
        }
    }
    Ok(())
}

// A symlink is not traversed when its target leaves the workspace root.
fn symlink_target(path: &Path, root_canon: &Path) -> Result<SymlinkTarget, std::io::Error> {
    match fs::canonicalize(path) {
        Ok(canon) if canon.starts_with(root_canon) => Ok(SymlinkTarget::Inside(canon)),
        Ok(_) => Ok(SymlinkTarget::Outside),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Ok(SymlinkTarget::Missing(error.to_string()))
        }
        Err(error) if is_symlink_loop(&error) => Ok(SymlinkTarget::Loop(error.to_string())),
        Err(error) => Err(error),
    }
}

fn ensure_absent(path: &Path) -> Result<(), WorkspaceError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(WorkspaceError {
            path: path.to_path_buf(),
            reason: "already exists".into(),
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_at(path, error)),
    }
}

fn refuse_symlink(path: &Path, root_canon: &Path) -> WorkspaceError {
    let reason = match symlink_target(path, root_canon) {
        Ok(SymlinkTarget::Outside) => {
            "refusing to follow symlink outside the workspace root".to_string()
        }
        Ok(SymlinkTarget::Inside(_) | SymlinkTarget::Missing(_)) => {
            "refusing to follow symlink".to_string()
        }
        Ok(SymlinkTarget::Loop(detail)) => format!("refusing to follow symlink: {detail}"),
        Err(error) => format!("refusing to follow symlink: {error}"),
    };
    WorkspaceError {
        path: path.to_path_buf(),
        reason,
    }
}

fn find_escaping_symlink(root_canon: &Path, dir: &Path) -> Result<(), WorkspaceError> {
    let entries = fs::read_dir(dir).map_err(|error| io_at(dir, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_at(dir, error))?;
        let path = entry.path();
        let kind = fs::symlink_metadata(&path)
            .map_err(|error| io_at(&path, error))?
            .file_type();
        if kind.is_symlink() {
            match symlink_target(&path, root_canon) {
                Ok(SymlinkTarget::Outside) => {
                    return Err(WorkspaceError {
                        path,
                        reason: "refusing to follow symlink outside the workspace root".into(),
                    });
                }
                Ok(SymlinkTarget::Loop(detail)) => {
                    return Err(WorkspaceError {
                        path,
                        reason: format!("refusing to follow symlink: {detail}"),
                    });
                }
                Ok(SymlinkTarget::Inside(_) | SymlinkTarget::Missing(_)) => {}
                Err(error) => return Err(io_at(&path, error)),
            }
        } else if kind.is_dir() {
            find_escaping_symlink(root_canon, &path)?;
        }
    }
    Ok(())
}

fn remove_tree(path: &Path) -> Result<(), WorkspaceError> {
    let meta = fs::symlink_metadata(path).map_err(|error| io_at(path, error))?;
    let kind = meta.file_type();
    if kind.is_symlink() || !kind.is_dir() {
        fs::remove_file(path).map_err(|error| io_at(path, error))?;
        return Ok(());
    }
    let entries = fs::read_dir(path).map_err(|error| io_at(path, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_at(path, error))?;
        remove_tree(&entry.path())?;
    }
    fs::remove_dir(path).map_err(|error| io_at(path, error))?;
    Ok(())
}

fn load_ignore(root: &Path) -> Result<Vec<IgnoreRule>, WorkspaceError> {
    let path = root.join(".gitignore");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_at(&path, error)),
    };
    let mut rules = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(rule) = parse_ignore_rule(line) {
            rules.push(rule);
        }
    }
    Ok(rules)
}

fn parse_ignore_rule(line: &str) -> Option<IgnoreRule> {
    if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
        return None;
    }
    if line.chars().any(|ch| matches!(ch, '*' | '?' | '[' | '\\')) {
        return None;
    }
    let dir_only = line.ends_with('/');
    let line = line.trim_end_matches('/');
    let explicit_anchor = line.starts_with('/');
    let line = line.trim_start_matches('/');
    if line.is_empty() {
        return None;
    }
    let mut path = PathBuf::new();
    for component in Path::new(line).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => path.push(part),
            _ => return None,
        }
    }
    if path.as_os_str().is_empty() {
        return None;
    }
    let anchored = explicit_anchor || path.components().nth(1).is_some();
    Some(IgnoreRule {
        path,
        anchored,
        dir_only,
    })
}

fn is_ignored(rules: &[IgnoreRule], root: &Path, path: &Path, is_dir: bool) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    if relative.as_os_str().is_empty() {
        return false;
    }
    rules.iter().any(|rule| rule.matches(relative, is_dir))
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
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("hyperion-{label}-{}-{n}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn names(tree: &FileTree) -> Vec<&str> {
        tree.entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

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

    #[test]
    fn create_file_and_folder_refresh_inside_root() {
        let tmp = Tmp::new("create");
        let root = tmp.path();
        let mut tree = FileTree::empty();
        let err = tree.create_file(Path::new("a.txt")).unwrap_err();
        assert_eq!(err.reason, "no folder is open");
        assert!(err.to_string().contains(&err.reason));

        let mut tree = FileTree::open(root.to_path_buf());
        tree.create_folder(&root.join("dir")).unwrap();
        tree.create_file(&root.join("dir").join("note.txt"))
            .unwrap();
        tree.create_file(&root.join("top.txt")).unwrap();
        fs::write(root.join("later.txt"), "later").unwrap();
        tree.refresh();

        assert!(tree.error.is_none(), "{:?}", tree.error);
        assert_eq!(fs::read_to_string(root.join("top.txt")).unwrap(), "");
        assert!(root.join("dir").is_dir());
        let dir = tree
            .entries
            .iter()
            .find(|entry| entry.name == "dir")
            .unwrap()
            .path
            .clone();
        assert!(dir.is_dir());
        tree.toggle_dir(&dir);
        assert!(names(&tree).contains(&"top.txt"));
        assert!(names(&tree).contains(&"later.txt"));
        assert!(names(&tree).contains(&"note.txt"));

        let missing = root.join("missing").join("a.txt");
        let err = tree.create_file(&missing).unwrap_err();
        assert_eq!(err.path, missing);
        assert!(
            err.reason.contains("os error") || err.reason.contains("No such file"),
            "{err}"
        );
        assert!(err.to_string().contains(&err.path.display().to_string()));
    }

    #[test]
    fn create_rejects_paths_outside_root() {
        let tmp = Tmp::new("create-root");
        let root = tmp.path().to_path_buf();
        let mut tree = FileTree::open(root.clone());
        let outside = Tmp::new("create-outside");
        let leaked_name = format!("leaked-{}", std::process::id());
        let leaked = root.parent().unwrap().join(&leaked_name);

        let err = tree
            .create_file(&root.join("..").join(&leaked_name))
            .unwrap_err();
        let created = leaked.exists();
        let _ = fs::remove_file(&leaked);
        assert!(!created, "{leaked:?} was created outside the root");
        assert_eq!(err.reason, "path escapes the workspace root");
        assert!(err.to_string().contains("escapes"));

        let err = tree.create_folder(outside.path()).unwrap_err();
        assert_eq!(err.reason, "path escapes the workspace root");
        assert!(outside.path().is_dir());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
            let err = tree
                .create_file(&root.join("link").join("pwn.txt"))
                .unwrap_err();
            assert_eq!(err.reason, "path escapes the workspace root");
            assert!(!outside.path().join("pwn.txt").exists());
            assert!(err.to_string().contains(&err.path.display().to_string()));
        }
    }

    #[test]
    fn rename_refreshes_and_rejects_escape_and_collision() {
        let tmp = Tmp::new("rename");
        let root = tmp.path();
        fs::write(root.join("a.txt"), "alpha").unwrap();
        fs::write(root.join("b.txt"), "beta").unwrap();
        let mut tree = FileTree::open(root.to_path_buf());
        tree.create_folder(&root.join("old")).unwrap();
        tree.create_file(&root.join("old").join("child.txt"))
            .unwrap();
        let old = tree
            .entries
            .iter()
            .find(|entry| entry.name == "old")
            .unwrap()
            .path
            .clone();
        tree.toggle_dir(&old);
        assert!(names(&tree).contains(&"child.txt"));

        tree.rename(&old, &root.join("new")).unwrap();
        tree.refresh();
        assert!(tree.error.is_none(), "{:?}", tree.error);
        assert!(!root.join("old").exists());
        assert_eq!(
            fs::read_to_string(root.join("new").join("child.txt")).unwrap(),
            ""
        );
        assert!(names(&tree).contains(&"new"));
        assert!(names(&tree).contains(&"child.txt"));
        assert!(!names(&tree).contains(&"old"));

        let err = tree
            .rename(&root.join("a.txt"), &root.join("b.txt"))
            .unwrap_err();
        assert_eq!(err.path, root.join("b.txt"));
        assert_eq!(err.reason, "already exists");
        assert!(err.to_string().contains(&err.path.display().to_string()));
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "alpha");
        assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "beta");

        let outside_name = format!("hyperion-renamed-{}", std::process::id());
        let leaked = root.parent().unwrap().join(&outside_name);
        let err = tree
            .rename(&root.join("a.txt"), &root.join("..").join(&outside_name))
            .unwrap_err();
        assert_eq!(err.reason, "path escapes the workspace root");
        assert!(!leaked.exists(), "{leaked:?}");
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "alpha");
        let _ = fs::remove_file(&leaked);
    }

    #[test]
    fn delete_file_refreshes_without_removing_directories() {
        let tmp = Tmp::new("delete-file");
        let root = tmp.path();
        fs::create_dir(root.join("dir")).unwrap();
        fs::write(root.join("dir").join("child.txt"), "child").unwrap();
        fs::write(root.join("gone.txt"), "bye").unwrap();
        let mut tree = FileTree::open(root.to_path_buf());

        let err = tree.delete_file(&root.join("dir")).unwrap_err();
        assert_eq!(err.path, root.join("dir"));
        assert_eq!(err.reason, "is a directory");
        assert!(root.join("dir").join("child.txt").is_file());

        tree.delete_file(&root.join("gone.txt")).unwrap();
        tree.refresh();
        assert!(!root.join("gone.txt").exists());
        assert!(!names(&tree).contains(&"gone.txt"));
        assert!(names(&tree).contains(&"dir"));

        let missing = root.join("gone.txt");
        let err = tree.delete_file(&missing).unwrap_err();
        assert_eq!(err.path, missing);
        assert!(
            err.reason.contains("os error") || err.reason.contains("No such file"),
            "{err}"
        );

        #[cfg(unix)]
        {
            let outside = Tmp::new("delete-file-outside");
            let keep = outside.path().join("keep.txt");
            fs::write(&keep, "safe").unwrap();
            std::os::unix::fs::symlink(&keep, root.join("link")).unwrap();
            tree.delete_file(&root.join("link")).unwrap();
            assert!(keep.is_file());
            assert!(!root.join("link").exists());
            assert_eq!(fs::read_to_string(&keep).unwrap(), "safe");
        }
    }

    #[test]
    fn delete_tree_removes_directories_and_refuses_escaping_symlinks() {
        let tmp = Tmp::new("delete-tree");
        let root = tmp.path().to_path_buf();
        let mut tree = FileTree::open(root.clone());
        fs::create_dir(root.join("box")).unwrap();
        fs::write(root.join("box").join("own.txt"), "own").unwrap();
        fs::create_dir(root.join("real")).unwrap();
        fs::write(root.join("real").join("keep.txt"), "keep").unwrap();

        let err = tree.delete_tree(&root).unwrap_err();
        assert_eq!(err.reason, "cannot delete the workspace root");
        assert!(root.join("real").join("keep.txt").is_file());

        let err = tree
            .delete_tree(&root.join("real").join("keep.txt"))
            .unwrap_err();
        assert_eq!(err.reason, "is not a directory");
        assert!(root.join("real").join("keep.txt").is_file());

        tree.delete_tree(&root.join("box")).unwrap();
        tree.refresh();
        assert!(!root.join("box").exists());
        assert!(!names(&tree).contains(&"box"));
        assert!(!names(&tree).contains(&"own.txt"));
        assert!(root.join("real").join("keep.txt").is_file());

        #[cfg(unix)]
        {
            let outside = Tmp::new("delete-tree-outside");
            fs::write(outside.path().join("keep.txt"), "safe").unwrap();
            std::os::unix::fs::symlink(outside.path(), root.join("linked")).unwrap();
            let err = tree.delete_tree(&root.join("linked")).unwrap_err();
            assert!(err.reason.contains("outside"), "{err}");
            assert!(err.to_string().contains(&err.path.display().to_string()));
            assert!(outside.path().join("keep.txt").is_file());
            assert!(root.join("linked").exists());

            fs::create_dir(root.join("nested")).unwrap();
            fs::write(root.join("nested").join("own.txt"), "own").unwrap();
            std::os::unix::fs::symlink(outside.path(), root.join("nested").join("out")).unwrap();
            let err = tree.delete_tree(&root.join("nested")).unwrap_err();
            assert!(err.reason.contains("outside"), "{err}");
            assert!(outside.path().join("keep.txt").is_file());
            assert!(root.join("nested").join("own.txt").is_file());

            std::os::unix::fs::symlink(&root.join("real"), root.join("alias")).unwrap();
            let err = tree.delete_tree(&root.join("alias")).unwrap_err();
            assert!(err.reason.contains("symlink"), "{err}");
            assert!(root.join("real").join("keep.txt").is_file());
            assert!(root.join("alias").exists());

            fs::create_dir(root.join("box2")).unwrap();
            fs::write(root.join("box2").join("own.txt"), "own").unwrap();
            std::os::unix::fs::symlink(&root.join("real"), root.join("box2").join("alias"))
                .unwrap();
            tree.delete_tree(&root.join("box2")).unwrap();
            assert!(!root.join("box2").exists());
            assert!(root.join("real").join("keep.txt").is_file());

            fs::create_dir(root.join("cyc")).unwrap();
            fs::write(root.join("cyc").join("keep.txt"), "k").unwrap();
            std::os::unix::fs::symlink("b", root.join("cyc").join("a")).unwrap();
            std::os::unix::fs::symlink("a", root.join("cyc").join("b")).unwrap();
            let err = tree.delete_tree(&root.join("cyc")).unwrap_err();
            assert!(
                err.reason.contains("symlink") || err.reason.contains("os error"),
                "{err}"
            );
            assert!(root.join("cyc").join("keep.txt").is_file());
        }
    }

    #[test]
    fn listing_skips_git_and_literal_gitignore() {
        let tmp = Tmp::new("ignore");
        let root = tmp.path();
        fs::write(
            root.join(".gitignore"),
            "\
# comment
secret.txt

*.log
foo*
artifacts/
logs/
",
        )
        .unwrap();
        fs::write(root.join("secret.txt"), "nope").unwrap();
        fs::write(root.join("foo.log"), "log").unwrap();
        fs::write(root.join("foo"), "foo").unwrap();
        fs::write(root.join("keep.txt"), "yes").unwrap();
        fs::write(root.join("logs"), "not a directory").unwrap();
        fs::create_dir(root.join("artifacts")).unwrap();
        fs::write(root.join("artifacts").join("inside.txt"), "hid").unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("secret.txt"), "nope").unwrap();
        fs::write(root.join("sub").join("keep.txt"), "yes").unwrap();

        let mut tree = FileTree::open(root.to_path_buf());
        assert!(tree.error.is_none(), "{:?}", tree.error);
        let top: Vec<_> = tree
            .entries
            .iter()
            .filter(|entry| entry.depth == 0)
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            top,
            vec!["sub", ".gitignore", "foo", "foo.log", "keep.txt", "logs"]
        );
        assert!(!names(&tree).contains(&".git"));
        assert!(!names(&tree).contains(&"secret.txt"));
        assert!(!names(&tree).contains(&"artifacts"));
        assert!(!names(&tree).contains(&"inside.txt"));
        assert!(!names(&tree).contains(&"HEAD"));

        let sub = tree
            .entries
            .iter()
            .find(|entry| entry.name == "sub")
            .unwrap()
            .path
            .clone();
        tree.toggle_dir(&sub);
        assert!(
            tree.entries
                .iter()
                .any(|entry| { entry.name == "keep.txt" && entry.depth == 1 })
        );
        assert!(tree.entries.iter().all(|entry| entry.name != "secret.txt"));
        assert!(tree.entries.iter().all(|entry| entry.name != ".git"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_walked_outside_or_in_a_loop() {
        let tmp = Tmp::new("symlink-walk");
        let root = tmp.path().to_path_buf();
        let outside = Tmp::new("symlink-outside");
        fs::write(outside.path().join("hidden.txt"), "no").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("keep.txt"), "yes").unwrap();
        std::os::unix::fs::symlink(".", root.join("sub").join("loop")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("out")).unwrap();

        let mut tree = FileTree::open(root.clone());
        assert!(tree.error.is_none(), "{:?}", tree.error);
        let sub = tree
            .entries
            .iter()
            .find(|entry| entry.name == "sub")
            .unwrap()
            .path
            .clone();
        tree.toggle_dir(&sub);
        let loop_entry = tree
            .entries
            .iter()
            .find(|entry| entry.name == "loop")
            .unwrap()
            .path
            .clone();
        tree.toggle_dir(&loop_entry);
        tree.toggle_dir(&root.join("out"));
        assert_eq!(
            tree.entries
                .iter()
                .filter(|entry| entry.name == "loop")
                .count(),
            1
        );
        assert!(tree.entries.iter().all(|entry| entry.depth < 4));
        assert!(tree.entries.iter().all(|entry| entry.name != "hidden.txt"));
        assert!(names(&tree).contains(&"keep.txt"));
        assert!(names(&tree).contains(&"out"));
    }
}
