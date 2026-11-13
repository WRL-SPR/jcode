//! `/cd` 명령의 경로 해석. 로컬 TUI와 서버가 같은 규칙을 쓰도록 한곳에 둔다.

use std::path::{Path, PathBuf};

/// `/cd` 인자를 실제로 존재하는 디렉터리의 절대 경로로 바꾼다.
///
/// - `~`, `~/x`, `~\x`는 `home` 기준으로 펼친다.
/// - 상대 경로는 `base`(현재 세션 작업 디렉터리) 기준으로 해석한다.
/// - 양 끝을 감싼 따옴표 한 쌍은 벗긴다 (공백이 든 경로를 붙여넣는 경우).
/// - 결과는 canonicalize하고, Windows의 `\\?\` 접두어는 일반 경로로 되돌린다.
///
/// 실패하면 사용자에게 그대로 보여줄 메시지를 돌려준다.
pub fn resolve_change_dir(
    input: &str,
    base: Option<&Path>,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    let raw = strip_matching_quotes(input.trim());
    if raw.is_empty() {
        return Err("Usage: /cd <path>".to_string());
    }

    let expanded = if raw == "~" {
        home.map(Path::to_path_buf)
            .ok_or_else(|| "Could not determine home directory for '~'".to_string())?
    } else if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        home.map(|home| home.join(rest))
            .ok_or_else(|| "Could not determine home directory for '~'".to_string())?
    } else {
        PathBuf::from(raw)
    };

    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        match base {
            Some(base) => base.join(expanded),
            None => {
                return Err(format!(
                    "Current working directory is unknown; use an absolute path instead of {raw}"
                ));
            }
        }
    };

    let canonical = std::fs::canonicalize(&candidate)
        .map_err(|_| format!("No such directory: {}", candidate.display()))?;
    if !canonical.is_dir() {
        return Err(format!("Not a directory: {}", canonical.display()));
    }
    Ok(simplify_verbatim_path(canonical))
}

fn strip_matching_quotes(value: &str) -> &str {
    for quote in ['"', '\''] {
        if value.len() >= 2
            && let Some(inner) = value
                .strip_prefix(quote)
                .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.trim();
        }
    }
    value
}

/// Windows canonicalize가 붙이는 `\\?\C:\...`, `\\?\UNC\...` 접두어를 걷어낸다.
/// 다른 플랫폼에서는 이런 문자열이 나오지 않으므로 그대로 둔다.
fn simplify_verbatim_path(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    simplify_verbatim_str(text).map_or(path, PathBuf::from)
}

fn simplify_verbatim_str(text: &str) -> Option<String> {
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{rest}"));
    }
    let rest = text.strip_prefix(r"\\?\")?;
    let mut chars = rest.chars();
    // 드라이브 문자 경로(`C:\...`)만 안전하게 되돌린다.
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => Some(rest.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod change_dir_tests {
    use super::*;

    fn canonical(path: &Path) -> PathBuf {
        simplify_verbatim_path(std::fs::canonicalize(path).unwrap())
    }

    #[test]
    fn change_dir_resolves_relative_path_against_base() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("child")).unwrap();

        let resolved = resolve_change_dir("child", Some(root.path()), None).unwrap();
        assert_eq!(resolved, canonical(&root.path().join("child")));

        let parent = resolve_change_dir("child/..", Some(root.path()), None).unwrap();
        assert_eq!(parent, canonical(root.path()));
    }

    #[test]
    fn change_dir_accepts_absolute_path_without_base() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().display().to_string();
        let resolved = resolve_change_dir(&input, None, None).unwrap();
        assert_eq!(resolved, canonical(root.path()));
    }

    #[test]
    fn change_dir_expands_tilde_from_home() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("proj")).unwrap();

        assert_eq!(
            resolve_change_dir("~", None, Some(home.path())).unwrap(),
            canonical(home.path())
        );
        assert_eq!(
            resolve_change_dir("~/proj", None, Some(home.path())).unwrap(),
            canonical(&home.path().join("proj"))
        );
        assert!(resolve_change_dir("~", None, None).is_err());
    }

    #[test]
    fn change_dir_strips_surrounding_quotes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("with space")).unwrap();
        let resolved = resolve_change_dir("\"with space\"", Some(root.path()), None).unwrap();
        assert_eq!(resolved, canonical(&root.path().join("with space")));
    }

    #[test]
    fn change_dir_rejects_missing_file_and_empty_inputs() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file.txt"), "x").unwrap();

        let missing = resolve_change_dir("nope", Some(root.path()), None).unwrap_err();
        assert!(missing.starts_with("No such directory"), "{missing}");
        let file = resolve_change_dir("file.txt", Some(root.path()), None).unwrap_err();
        assert!(file.starts_with("Not a directory"), "{file}");
        assert!(resolve_change_dir("   ", Some(root.path()), None).is_err());
        assert!(resolve_change_dir("relative", None, None).is_err());
    }

    #[test]
    fn change_dir_simplifies_windows_verbatim_prefixes() {
        assert_eq!(
            simplify_verbatim_str(r"\\?\C:\work\repo").as_deref(),
            Some(r"C:\work\repo")
        );
        assert_eq!(
            simplify_verbatim_str(r"\\?\UNC\server\share\dir").as_deref(),
            Some(r"\\server\share\dir")
        );
        assert_eq!(simplify_verbatim_str(r"\\?\Volume{abc}\dir"), None);
        assert_eq!(simplify_verbatim_str("/home/user"), None);
    }
}
