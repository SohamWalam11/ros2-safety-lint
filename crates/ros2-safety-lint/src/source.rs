//! Source files held in memory, with byte-offset → line/column mapping.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Index into [`SourceMap::files`].
pub type FileId = usize;

/// A byte range inside one source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Loc {
    pub file: FileId,
    pub start: usize,
    pub end: usize,
}

impl Loc {
    pub fn new(file: FileId, start: usize, end: usize) -> Self {
        Loc {
            file,
            start,
            end: end.max(start),
        }
    }
}

/// One file's text plus its precomputed line starts.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Path relative to the workspace root, with `/` separators.
    pub rel_path: String,
    /// Absolute (or root-joined) path on disk.
    pub abs_path: PathBuf,
    pub text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(rel_path: String, abs_path: PathBuf, text: String) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        SourceFile {
            rel_path,
            abs_path,
            text,
            line_starts,
        }
    }

    /// 1-based line and 1-based column (in characters) for a byte offset.
    pub fn line_col(&self, byte: usize) -> (usize, usize) {
        let byte = byte.min(self.text.len());
        let line_idx = match self.line_starts.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let line_start = self.line_starts[line_idx];
        let col = self.text[line_start..floor_char_boundary(&self.text, byte)]
            .chars()
            .count();
        (line_idx + 1, col + 1)
    }

    /// Text of a 1-based line without its newline.
    pub fn line_text(&self, line: usize) -> &str {
        if line == 0 || line > self.line_starts.len() {
            return "";
        }
        let start = self.line_starts[line - 1];
        let end = self
            .line_starts
            .get(line)
            .copied()
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn slice(&self, loc: Loc) -> &str {
        let s = floor_char_boundary(&self.text, loc.start);
        let e = floor_char_boundary(&self.text, loc.end.max(loc.start));
        &self.text[s..e]
    }
}

pub fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// All files loaded for one analysis run.
#[derive(Debug, Default, Clone)]
pub struct SourceMap {
    pub files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn add(&mut self, f: SourceFile) -> FileId {
        self.files.push(f);
        self.files.len() - 1
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        &self.files[id]
    }

    pub fn find_by_abs(&self, p: &Path) -> Option<FileId> {
        let norm = normalize_path(p);
        self.files
            .iter()
            .position(|f| normalize_path(&f.abs_path) == norm)
    }

    pub fn find_by_rel(&self, rel: &str) -> Option<FileId> {
        self.files.iter().position(|f| f.rel_path == rel)
    }
}

/// Lexically normalizes a path (resolves `.` and `..`, unifies separators, lowercases
/// on Windows) without touching the filesystem.
pub fn normalize_path(p: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    let s = p.to_string_lossy().replace('\\', "/");
    let absolute = s.starts_with('/');
    for comp in s.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c.to_string()),
        }
    }
    let joined = parts.join("/");
    let out = if absolute {
        format!("/{joined}")
    } else {
        joined
    };
    if cfg!(windows) {
        out.to_lowercase()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_basic() {
        let f = SourceFile::new("a".into(), "a".into(), "ab\ncd\n".into());
        assert_eq!(f.line_col(0), (1, 1));
        assert_eq!(f.line_col(3), (2, 1));
        assert_eq!(f.line_col(4), (2, 2));
        assert_eq!(f.line_text(2), "cd");
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_path(Path::new("a/./b/../c")), "a/c");
    }
}
