use std::fs;
use std::path::{Path, PathBuf};

const ROOT_FILES: &[&str] = &["Cargo.toml", "deny.toml", "rustfmt.toml"];
const TREES: &[&str] = &["crates", "deploy", "tools/otp-relay"];
const SKIPPED_DIRS: &[&str] = &["target", "node_modules", "__pycache__", "migrations"];
const SKIPPED_FILES: &[&str] = &["tools/otp-relay/contrib/config.example.toml"];
const SKIPPED_EXTENSIONS: &[&str] = &["md", "json", "py", "sql", "lock"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_comments(src: &str) -> Vec<usize> {
    let b = src.as_bytes();
    let (mut i, mut line, mut found) = (0, 1, Vec::new());
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while i < b.len() {
        match b[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                found.push(line);
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                found.push(line);
                let mut depth = 0;
                while i < b.len() {
                    if b[i..].starts_with(b"/*") {
                        depth += 1;
                        i += 2;
                    } else if b[i..].starts_with(b"*/") {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        if b[i] == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                }
            }
            b'r' | b'b' if i == 0 || !ident(b[i - 1]) => {
                let mut j = i + 1;
                if b[i] == b'b' && b.get(j) == Some(&b'r') {
                    j += 1;
                }
                let hashes = b[j..].iter().take_while(|&&c| c == b'#').count();
                let raw = b[i] == b'r' || j > i + 1;
                if raw && b.get(j + hashes) == Some(&b'"') {
                    let mut end = vec![b'"'];
                    end.extend(std::iter::repeat_n(b'#', hashes));
                    i = j + hashes + 1;
                    while i < b.len() && !b[i..].starts_with(&end) {
                        if b[i] == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                    i += end.len();
                } else {
                    i += 1;
                    while i < b.len() && ident(b[i]) {
                        i += 1;
                    }
                }
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    if b.get(i) == Some(&b'\n') {
                        line += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'\'' => {
                let rest = &src[i + 1..];
                let mut chars = rest.char_indices();
                match chars.next() {
                    Some((_, '\\')) => {
                        let close =
                            rest.get(2..).and_then(|r| r.find('\'')).map_or(rest.len(), |k| k + 3);
                        i += 1 + close;
                    }
                    Some((_, c)) if chars.next().map(|(_, n)| n) == Some('\'') => {
                        i += 2 + c.len_utf8();
                    }
                    _ => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    found
}

fn hash_comments(src: &str, shell: bool) -> Vec<usize> {
    let mut found = Vec::new();
    let mut quote: Option<char> = None;
    let mut heredoc: Option<String> = None;
    for (n, text) in src.lines().enumerate() {
        if let Some(end) = &heredoc {
            if text.trim() == end {
                heredoc = None;
            }
            continue;
        }
        if n == 0 && text.starts_with("#!") {
            continue;
        }
        if !shell {
            quote = None;
        }
        let chars: Vec<char> = text.chars().collect();
        let mut k = 0;
        while k < chars.len() {
            let c = chars[k];
            match quote {
                Some(q) if c == '\\' && q == '"' => k += 1,
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if c == '\'' || c == '"' => quote = Some(c),
                None if c == '#' && (k == 0 || matches!(chars[k - 1], ' ' | '\t' | ';' | '(')) => {
                    let rest: String = chars[k..].iter().collect();
                    if !(shell && rest.starts_with("# shellcheck ")) {
                        found.push(n + 1);
                    }
                    break;
                }
                None => {}
            }
            k += 1;
        }
        if shell
            && quote.is_none()
            && let Some(pos) = text.find("<<")
        {
            let tag: String = text[pos + 2..]
                .trim_start_matches('-')
                .trim_start()
                .chars()
                .filter(|c| !matches!(c, '\'' | '"'))
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !tag.is_empty() {
                heredoc = Some(tag);
            }
        }
    }
    found
}

fn comments(path: &Path, src: &str) -> Vec<usize> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let first = src.lines().next().unwrap_or("");
    match ext {
        "rs" => rust_comments(src),
        "sh" => hash_comments(src, true),
        _ if first.starts_with("#!") && first.ends_with("sh") => hash_comments(src, true),
        _ => hash_comments(src, false),
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                walk(&path, out);
            }
        } else {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !SKIPPED_EXTENSIONS.contains(&ext) {
                out.push(path);
            }
        }
    }
}

#[test]
fn no_comments_outside_apps() {
    let root = repo_root();
    let mut files: Vec<PathBuf> = ROOT_FILES.iter().map(|f| root.join(f)).collect();
    for tree in TREES {
        walk(&root.join(tree), &mut files);
    }
    let mut offenders = Vec::new();
    for path in &files {
        let rel = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        if SKIPPED_FILES.contains(&rel.as_str()) {
            continue;
        }
        let src = fs::read_to_string(path).unwrap();
        offenders.extend(comments(path, &src).into_iter().map(|l| format!("{rel}:{l}")));
    }
    assert!(files.len() > 100, "{} fichiers seulement : mauvaise racine ?", files.len());
    assert!(offenders.is_empty(), "commentaires interdits :\n{}", offenders.join("\n"));
}

#[test]
fn rust_lexer() {
    let src = "let a = \"// pas\";\nlet b = r#\"/* pas \"# ;\nlet c = '\\'';\nfn f<'a>(x: &'a str) {}\nlet d = '\"'; // oui\n/* oui /* imbriqué */ encore */\nlet e = b'/'; let r#type = 1; //oui\n";
    assert_eq!(rust_comments(src), vec![5, 6, 7]);
}

#[test]
fn hash_lexer() {
    let shell = "#!/bin/sh\necho \"# pas\" '# pas' $# ${#x}\n# oui\n  # shellcheck disable=SC2086\nx=1 # oui\ncat <<'EOF'\n# pas\nEOF\nprintf 'a\n# pas\nb'\n";
    assert_eq!(hash_comments(shell, true), vec![3, 5]);
    let toml = "a = \"#pas\" # oui\n# oui\nb = 'c#d'\n";
    assert_eq!(hash_comments(toml, false), vec![1, 2]);
}
