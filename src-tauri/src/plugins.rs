//! Widgets from a plugin folder: `%APPDATA%\Oriel\widgets\name.js`, with an optional `name.css`.
//!
//! The page loads them through the `oriel-plugins` URI scheme, which on Windows WebView2 is
//! `http://oriel-plugins.localhost/name.js`. Nothing is served unless the setting is on, and
//! only plain `.js` and `.css` files directly inside the folder: no subfolders, no `..`, no
//! symlinks leading out, nothing over `MAX_BYTES`.

use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::http::{header, Response, StatusCode};

pub const SCHEME: &str = "oriel-plugins";
/// A widget is a small script; this is far more than it needs.
pub const MAX_BYTES: u64 = 512 * 1024;
/// How many widgets are listed, so a folder of thousands of files can't flood the page.
pub const MAX_FILES: usize = 64;
const MAX_NAME: usize = 64;

pub fn dir() -> PathBuf {
    crate::settings::path().with_file_name("widgets")
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Js,
    Css,
}

impl Kind {
    fn content_type(self) -> &'static str {
        match self {
            Kind::Js => "text/javascript; charset=utf-8",
            Kind::Css => "text/css; charset=utf-8",
        }
    }
}

/// `/clock.js` -> the file name and its kind, if it's a plain name that ends in .js or .css.
/// Anything that could mean a different place (separators, `..`, a drive, percent-escapes,
/// an NTFS stream) is refused here, before the file system is touched.
pub fn parse_name(path: &str) -> Option<(&str, Kind)> {
    let name = path.strip_prefix('/').unwrap_or(path);
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if name.is_empty() || name.len() > MAX_NAME || name.starts_with('.') || name.contains("..") || !name.chars().all(plain) {
        return None;
    }
    let kind = match name.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
        "js" => Kind::Js,
        "css" => Kind::Css,
        _ => return None,
    };
    Some((name, kind))
}

#[derive(Debug, PartialEq)]
pub enum Refused {
    /// Not a name we serve, or no such file.
    NotFound,
    TooBig,
}

/// The file `path` names inside `dir`, if serving it is allowed.
pub fn resolve(dir: &Path, path: &str) -> Result<(PathBuf, Kind), Refused> {
    let (name, kind) = parse_name(path).ok_or(Refused::NotFound)?;
    let root = dir.canonicalize().map_err(|_| Refused::NotFound)?;
    // Canonicalizing follows symlinks and junctions: if the real file isn't directly in the real
    // folder, the name led somewhere else.
    let file = root.join(name).canonicalize().map_err(|_| Refused::NotFound)?;
    if file.parent() != Some(root.as_path()) {
        return Err(Refused::NotFound);
    }
    let meta = std::fs::metadata(&file).map_err(|_| Refused::NotFound)?;
    if !meta.is_file() {
        return Err(Refused::NotFound);
    }
    if meta.len() > MAX_BYTES {
        return Err(Refused::TooBig);
    }
    Ok((file, kind))
}

fn reply(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        // Edited files must show up after a restart, not from the page's cache.
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        // Lets the page read an error's message and line from a script loaded across origins.
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body)
        .unwrap()
}

/// The scheme handler. With the setting off it answers 404 to everything.
pub fn serve(dir: &Path, enabled: bool, method: &str, path: &str) -> Response<Vec<u8>> {
    let not_found = || reply(StatusCode::NOT_FOUND, "text/plain; charset=utf-8", b"not found".to_vec());
    if !enabled || !matches!(method, "GET" | "HEAD") {
        return not_found();
    }
    match resolve(dir, path) {
        Ok((file, kind)) => match std::fs::read(file) {
            Ok(body) => reply(StatusCode::OK, kind.content_type(), if method == "HEAD" { Vec::new() } else { body }),
            Err(_) => not_found(),
        },
        Err(Refused::TooBig) => reply(StatusCode::PAYLOAD_TOO_LARGE, "text/plain; charset=utf-8", format!("over {} KB", MAX_BYTES / 1024).into_bytes()),
        Err(Refused::NotFound) => not_found(),
    }
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Plugin {
    /// File name, e.g. `clock.js`
    pub file: String,
    pub url: String,
    /// The style sheet beside it, if there is one
    pub css: Option<String>,
}

/// What the page should load: every servable `.js` file, with its `.css`. Empty when off.
pub fn list(dir: &Path, enabled: bool) -> Vec<Plugin> {
    if !enabled {
        return Vec::new();
    }
    let url = |name: &str| format!("http://{SCHEME}.localhost/{name}");
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
    names.sort_by_key(|n| n.to_ascii_lowercase());
    let ok = |name: &str, want: Kind| matches!(parse_name(name), Some((_, k)) if k == want) && resolve(dir, name).is_ok();
    names
        .iter()
        .filter(|n| ok(n, Kind::Js))
        .take(MAX_FILES)
        .map(|js| {
            let css = format!("{}.css", &js[..js.len() - 3]);
            let css = names.iter().find(|n| n.eq_ignore_ascii_case(&css)).filter(|n| ok(n, Kind::Css));
            Plugin { file: js.clone(), url: url(js), css: css.map(|c| url(c)) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own under the system temp dir, removed afterwards.
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let p = std::env::temp_dir().join(format!("oriel-plugins-test-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn plain_names_only() {
        assert_eq!(parse_name("/clock.js"), Some(("clock.js", Kind::Js)));
        assert_eq!(parse_name("/World-Clock_2.CSS"), Some(("World-Clock_2.CSS", Kind::Css)));
        for bad in [
            "", "/", "/clock", "/clock.json", "/clock.js.txt", "/.js", "/.hidden.js", "/../clock.js", "/..\\clock.js", "/a/b.js", "/a\\b.js",
            "//clock.js", "/C:/clock.js", "/C:clock.js", "/clock.js:stream", "/clock%2ejs", "/cl%00ock.js", "/clock .js", "/clock.js ", "/a..b.js",
            "/con\u{202e}js.js",
        ] {
            assert_eq!(parse_name(bad), None, "{bad:?} should be refused");
        }
        assert_eq!(parse_name(&format!("/{}.js", "a".repeat(MAX_NAME))), None);
    }

    #[test]
    fn serves_files_directly_in_the_folder() {
        let t = Tmp::new("serve");
        std::fs::write(t.0.join("clock.js"), "Oriel.register('clock', {});").unwrap();
        std::fs::write(t.0.join("clock.css"), ".clock { color: red }").unwrap();
        std::fs::write(t.0.join("notes.txt"), "no").unwrap();
        std::fs::create_dir(t.0.join("sub.js")).unwrap();
        std::fs::create_dir(t.0.join("sub")).unwrap();
        std::fs::write(t.0.join("sub").join("inner.js"), "x").unwrap();

        let r = serve(&t.0, true, "GET", "/clock.js");
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "text/javascript; charset=utf-8");
        assert_eq!(r.body(), b"Oriel.register('clock', {});");
        let r = serve(&t.0, true, "GET", "/clock.css");
        assert_eq!(r.headers()[header::CONTENT_TYPE], "text/css; charset=utf-8");
        assert!(serve(&t.0, true, "HEAD", "/clock.js").body().is_empty());

        for path in ["/notes.txt", "/sub.js", "/sub/inner.js", "/sub\\inner.js", "/../clock.js", "/missing.js", "/"] {
            assert_eq!(serve(&t.0, true, "GET", path).status(), StatusCode::NOT_FOUND, "{path}");
        }
        assert_eq!(serve(&t.0, true, "POST", "/clock.js").status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn nothing_when_off() {
        let t = Tmp::new("off");
        std::fs::write(t.0.join("clock.js"), "x").unwrap();
        assert_eq!(serve(&t.0, false, "GET", "/clock.js").status(), StatusCode::NOT_FOUND);
        assert!(list(&t.0, false).is_empty());
    }

    #[test]
    fn size_cap() {
        let t = Tmp::new("size");
        std::fs::write(t.0.join("ok.js"), vec![b' '; MAX_BYTES as usize]).unwrap();
        std::fs::write(t.0.join("big.js"), vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert_eq!(serve(&t.0, true, "GET", "/ok.js").status(), StatusCode::OK);
        assert_eq!(serve(&t.0, true, "GET", "/big.js").status(), StatusCode::PAYLOAD_TOO_LARGE);
        let files: Vec<_> = list(&t.0, true).into_iter().map(|p| p.file).collect();
        assert_eq!(files, ["ok.js"]);
    }

    #[test]
    fn a_missing_folder_is_just_empty() {
        let gone = std::env::temp_dir().join("oriel-plugins-test-never-made");
        assert_eq!(resolve(&gone, "/a.js"), Err(Refused::NotFound));
        assert!(list(&gone, true).is_empty());
    }

    #[test]
    fn symlinks_cannot_lead_out() {
        let t = Tmp::new("link");
        let folder = t.0.join("widgets");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(t.0.join("secret.js"), "outside").unwrap();
        std::fs::write(folder.join("real.js"), "inside").unwrap();
        // Creating symlinks needs developer mode or admin on Windows; where that isn't allowed there's nothing to test.
        #[cfg(windows)]
        use std::os::windows::fs::symlink_file as link;
        #[cfg(not(windows))]
        use std::os::unix::fs::symlink as link;
        if link(t.0.join("secret.js"), folder.join("out.js")).is_err() {
            eprintln!("skipped: can't create symlinks here");
            return;
        }
        link(folder.join("real.js"), folder.join("alias.js")).unwrap();
        assert_eq!(serve(&folder, true, "GET", "/out.js").status(), StatusCode::NOT_FOUND);
        assert_eq!(serve(&folder, true, "GET", "/alias.js").status(), StatusCode::OK); // stays inside
        let files: Vec<_> = list(&folder, true).into_iter().map(|p| p.file).collect();
        assert_eq!(files, ["alias.js", "real.js"]);
    }

    #[test]
    fn lists_scripts_with_their_style_sheets() {
        let t = Tmp::new("list");
        for f in ["b.js", "a.js", "a.css", "c.css", "d.txt", "e.json", "bad name.js", ".hidden.js"] {
            std::fs::write(t.0.join(f), "x").unwrap();
        }
        let got = list(&t.0, true);
        let url = |n: &str| format!("http://oriel-plugins.localhost/{n}");
        assert_eq!(
            got,
            [
                Plugin { file: "a.js".into(), url: url("a.js"), css: Some(url("a.css")) },
                Plugin { file: "b.js".into(), url: url("b.js"), css: None },
            ]
        );
    }
}
