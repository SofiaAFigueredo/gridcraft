//! Which hyperlink targets may be opened. A cell's link comes from the document (often a file
//! somebody sent), so it is hostile input: only `http`, `https` and `mailto` addresses and files
//! or folders on this computer ever leave the workbook for the system; `smb:`, UNC paths, custom
//! URL handlers, `javascript:` and the like are refused. A local target is opened only after the
//! user confirms it, only when it exists, and never when it is a program or script
//! ([`resolve_local`]). Everything else without a scheme stays an in-workbook reference.

use std::path::{Path, PathBuf};

use gridcraft_core::{CellRef, RangeRef};
use gridcraft_formula::Expr;
use gridcraft_model::Workbook;

/// What a link target is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// A cell, range, name or table of the workbook (`Sheet2!A1`, `Total`).
    Internal(String),
    /// An address to hand to the system, trimmed: `http`, `https` or `mailto` only.
    External(String),
    /// A file or folder on this computer (a `file:` address decoded, or a path), relative to the
    /// workbook when it isn't absolute. Opened only after confirmation ([`resolve_local`]).
    Local(String),
    /// Refused, with the reason to show.
    Refused(String),
}

const ALLOWED: [&str; 3] = ["http", "https", "mailto"];

/// Classifies a link target. Never panics; the single place that decides what may be opened.
pub fn classify(target: &str) -> LinkKind {
    let t = target.trim();
    if t.chars().any(char::is_control) {
        return LinkKind::Refused("The link contains control characters.".into());
    }
    if t.starts_with("\\\\") || t.starts_with("//") {
        return LinkKind::Refused("Links to network locations aren't opened.".into());
    }
    if RangeRef::parse(t).is_some() {
        return LinkKind::Internal(t.to_string());
    }
    if let Some(path) = path_like(t) {
        return local(path);
    }
    let Some((scheme, rest)) = t.split_once(':') else { return LinkKind::Internal(t.to_string()) };
    let scheme_like = scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_like {
        return LinkKind::Internal(t.to_string());
    }
    let lower = scheme.to_ascii_lowercase();
    if lower == "file" {
        return match file_url_path(rest) {
            Ok(path) => local(&path),
            Err(why) => LinkKind::Refused(why),
        };
    }
    if ALLOWED.contains(&lower.as_str()) && !rest.trim().is_empty() {
        LinkKind::External(t.to_string())
    } else {
        LinkKind::Refused(format!("Links of type \"{lower}:\" aren't opened. Only http, https and mailto links are allowed."))
    }
}

/// Classifies the target of a stored hyperlink. `external` is the workbook's own mark that the
/// link leaves the workbook ("Existing File or Web Page"), which makes a bare file name such as
/// `notes.txt` a file instead of a defined name.
pub fn classify_link(target: &str, external: bool) -> LinkKind {
    match classify(target) {
        LinkKind::Internal(t)
            if external && !t.trim().is_empty() && RangeRef::parse(&t).is_none() && !t.starts_with('#') && !(t.contains('!') && !t.contains('#')) =>
        {
            local(t.split('#').next().unwrap_or_default())
        }
        other => other,
    }
}

/// The text of a path that is plainly one: a drive (`C:\x`), an absolute, home or `.`/`..`
/// relative path, or anything with a separator that is not a `Sheet!A1` reference. Sheet and
/// defined names can't hold `/` or `\`, so those are never mistaken for references.
fn path_like(t: &str) -> Option<&str> {
    let b = t.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\');
    let rooted =
        t.starts_with('/') || t.starts_with("~/") || t.starts_with("./") || t.starts_with("../") || t.starts_with(".\\") || t.starts_with("..\\");
    // `file:/x`, `smb:/h/s` and `javascript:a/b` carry a scheme: `classify` must see them. A single
    // letter is a drive, not a scheme.
    let has_scheme = t.split_once(':').is_some_and(|(s, _)| {
        s.len() > 1
            && s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    let separated = t.contains(['/', '\\']) && !t.contains('!') && !has_scheme;
    (drive || rooted || separated).then(|| t.split('#').next().unwrap_or(t))
}

/// The path of a `file:` address: no host (or `localhost`), percent-decoded. Another host is a
/// network location and is refused.
fn file_url_path(rest: &str) -> Result<String, String> {
    let rest = rest.split(['#', '?']).next().unwrap_or_default();
    let path = match rest.strip_prefix("//") {
        Some(after) => {
            let (host, path) = after.split_at(after.find('/').unwrap_or(after.len()));
            if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
                return Err("Links to network locations aren't opened.".into());
            }
            path
        }
        None => rest,
    };
    let decoded = percent_decode(path).ok_or_else(|| "The link isn't a valid file address.".to_string())?;
    // `file:///C:/x` carries the drive after a slash.
    let b = decoded.as_bytes();
    let decoded = if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' { decoded[1..].to_string() } else { decoded };
    if decoded.is_empty() || decoded.chars().any(char::is_control) || decoded.starts_with("//") || decoded.starts_with("\\\\") {
        return Err("The link isn't a valid file address.".into());
    }
    Ok(decoded)
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn local(path: &str) -> LinkKind {
    let path = path.trim();
    if path.is_empty() {
        return LinkKind::Refused("The link has no file or folder.".into());
    }
    if path.starts_with("\\\\") || path.starts_with("//") {
        return LinkKind::Refused("Links to network locations aren't opened.".into());
    }
    if is_program(Path::new(path)) {
        return LinkKind::Refused("Links to programs and scripts aren't opened. Open the file from your file manager if you trust it.".into());
    }
    LinkKind::Local(path.to_string())
}

/// Extensions that run code when the system opens them. Compared without case and without the
/// trailing dots and spaces Windows ignores in a name.
const PROGRAMS: &[&str] = &[
    "exe",
    "com",
    "bat",
    "cmd",
    "msi",
    "msp",
    "scr",
    "pif",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "vb",
    "js",
    "jse",
    "wsf",
    "wsh",
    "hta",
    "lnk",
    "url",
    "jar",
    "jnlp",
    "app",
    "command",
    "sh",
    "bash",
    "zsh",
    "csh",
    "ksh",
    "desktop",
    "run",
    "bin",
    "appimage",
    "dll",
    "ocx",
    "drv",
    "sys",
    "vxd",
    "wcx",
    "cpl",
    "msc",
    "reg",
    "inf",
    "scf",
    "chm",
    "rdp",
    "website",
    "gadget",
    "application",
    "appref-ms",
    "appcontent-ms",
    "settingcontent-ms",
    "appx",
    "appxbundle",
    "action",
    "workflow",
    "terminal",
    "py",
    "pyw",
    "pl",
    "rb",
    "pkg",
    "dmg",
];

fn is_program(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let name = name.trim_end_matches(['.', ' ']);
    let Some((_, ext)) = name.rsplit_once('.') else { return false };
    let ext = ext.trim().to_ascii_lowercase();
    PROGRAMS.contains(&ext.as_str())
}

/// The file or folder a [`LinkKind::Local`] names, ready for the system: relative paths are taken
/// from the workbook's folder (`base`, the folder of the saved file), `~/` from the home folder.
/// Refuses what doesn't exist, programs and scripts (by name, by what a link resolves to, and on
/// Unix by the executable bit), so a confirmed click can only open documents and folders.
pub fn resolve_local(base: Option<&Path>, target: &str) -> Result<PathBuf, String> {
    let path = match target.strip_prefix("~/") {
        Some(rest) => {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
            home.ok_or_else(|| "The home folder isn't known.".to_string())?.join(rest)
        }
        None => PathBuf::from(target),
    };
    let path = if path.is_absolute() || path.has_root() || target.as_bytes().get(1) == Some(&b':') {
        path
    } else {
        let base = base.ok_or_else(|| "Save the workbook first: this link is relative to the workbook's folder.".to_string())?;
        base.join(path)
    };
    if is_program(&path) {
        return Err("Links to programs and scripts aren't opened.".into());
    }
    let real = std::fs::canonicalize(&path).map_err(|_| format!("\"{}\" doesn't exist or can't be reached.", path.display()))?;
    if is_program(&real) {
        return Err("Links to programs and scripts aren't opened.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&real).map_err(|e| e.to_string())?;
        if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
            return Err("Links to programs and scripts aren't opened.".into());
        }
    }
    Ok(path)
}

/// The target of the link in a cell: its stored hyperlink, or the first argument of a cell whose
/// formula is `=HYPERLINK(link, …)`, evaluated. `external` is the stored link's mark ([`classify_link`]).
pub fn link_at(wb: &Workbook, sheet: usize, at: CellRef) -> Option<(String, bool)> {
    let sh = wb.sheet(sheet)?;
    if let Some(h) = sh.hyperlinks.get(&at) {
        return Some((h.target.clone(), h.external));
    }
    let expr = sh.cell(at)?.formula.as_ref()?.expr()?;
    let Expr::Call(name, args) = &expr else { return None };
    if name != "HYPERLINK" {
        return None;
    }
    match gridcraft_calc::recalc::evaluate_expr(wb, sheet, at, args.first()?) {
        gridcraft_core::Value::Text(t) if !t.trim().is_empty() => Some((t.to_string(), true)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_https_and_mailto_leave_the_workbook() {
        for ok in ["http://a.example/x", "HTTPS://A.example", "  https://a.example  ", "MailTo:a@b.example"] {
            assert!(matches!(classify(ok), LinkKind::External(_)), "{ok}");
        }
        assert_eq!(classify(" https://a.example "), LinkKind::External("https://a.example".into()));
        for bad in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "file://host/etc/passwd",
            "smb://host/share",
            "ms-excel:ofe|u|http://x",
            "\\\\host\\share\\x.exe",
            "//host/share",
            "C:\\Windows\\calc.exe",
            "\tjavascript:1",
            "java\nscript:1",
            "\u{1}http://a.example",
            "http:",
            "mailto:",
            "x-custom://a",
        ] {
            assert!(matches!(classify(bad), LinkKind::Refused(_)), "{bad:?} -> {:?}", classify(bad));
        }
    }

    #[test]
    fn local_files_and_folders_are_recognised() {
        for (target, path) in [
            ("file:///home/u/a%20b.pdf", "/home/u/a b.pdf"),
            ("FILE://localhost/tmp/x.txt", "/tmp/x.txt"),
            ("file:///C:/Users/u/doc.txt", "C:/Users/u/doc.txt"),
            ("file:///tmp/folder/", "/tmp/folder/"),
            ("file:/tmp/a.txt", "/tmp/a.txt"),
            ("/tmp/a.txt", "/tmp/a.txt"),
            ("C:\\Docs\\a.pdf", "C:\\Docs\\a.pdf"),
            ("docs/a.pdf", "docs/a.pdf"),
            ("./a.pdf", "./a.pdf"),
            ("..\\a.pdf", "..\\a.pdf"),
            ("~/Documents", "~/Documents"),
            ("Book.xlsx#Sheet1!A2", "Book.xlsx"),
            ("/tmp/Book.xlsx#Sheet1!A2", "/tmp/Book.xlsx"),
        ] {
            let kind = classify(target);
            if target == "Book.xlsx#Sheet1!A2" {
                // A bare name is still told from a defined name only by the workbook's mark.
                assert!(matches!(kind, LinkKind::Internal(_)), "{target}");
                assert_eq!(classify_link(target, true), LinkKind::Local(path.into()), "{target}");
            } else {
                assert_eq!(kind, LinkKind::Local(path.into()), "{target}");
            }
        }
        assert_eq!(classify_link("notes.txt", true), LinkKind::Local("notes.txt".into()));
        assert!(matches!(classify_link("notes.txt", false), LinkKind::Internal(_)));
        assert!(matches!(classify_link("Total", false), LinkKind::Internal(_)));
        assert!(matches!(classify_link("Sheet2!A1", true), LinkKind::Internal(_)));
        assert!(matches!(classify_link("B2", true), LinkKind::Internal(_)));
    }

    #[test]
    fn local_programs_network_shares_and_bad_addresses_are_refused() {
        for bad in [
            "file://host/share/x.txt",
            "file:////host/share/x.txt",
            "file:///tmp/run.sh",
            "file:///tmp/run.SH",
            "/usr/bin/tool.exe",
            "C:\\x\\setup.msi",
            "docs/macro.vbs",
            "docs/app.exe . ",
            "~/Desktop/Tool.app",
            "file:///tmp/%00.txt",
            "file:///tmp/%zz",
            "file:///tmp/%ff.txt",
            "file:",
            "smb:/h/s",
            "javascript:a/b",
            "docs/setup.jnlp",
            "docs/Snap.MSC",
            "docs/tool.settingcontent-ms",
        ] {
            assert!(matches!(classify(bad), LinkKind::Refused(_)), "{bad:?} -> {:?}", classify(bad));
        }
        assert!(matches!(classify_link("tool.exe", true), LinkKind::Refused(_)));
    }

    #[test]
    fn resolving_a_local_link_needs_an_existing_document_or_folder() {
        #[allow(clippy::disallowed_methods)] // native-only test: a unique temp dir name
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("gridcraft-links-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("doc.txt"), "x").unwrap();
        std::fs::write(dir.join("prog"), "#!/bin/sh\n").unwrap();
        // A relative link is taken from the workbook's folder; absolute ones and folders open as they are.
        assert_eq!(resolve_local(Some(&dir), "doc.txt").unwrap(), dir.join("doc.txt"));
        assert_eq!(resolve_local(Some(&dir), "sub").unwrap(), dir.join("sub"));
        assert_eq!(resolve_local(None, dir.join("doc.txt").to_str().unwrap()).unwrap(), dir.join("doc.txt"));
        assert!(resolve_local(None, "doc.txt").unwrap_err().contains("Save the workbook"));
        assert!(resolve_local(Some(&dir), "missing.txt").unwrap_err().contains("doesn't exist"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Programs by the executable bit, and through a link with an innocent name.
            std::fs::set_permissions(dir.join("prog"), std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(resolve_local(Some(&dir), "prog").is_err());
            std::os::unix::fs::symlink(dir.join("prog"), dir.join("looks-fine.txt")).unwrap();
            assert!(resolve_local(Some(&dir), "looks-fine.txt").is_err());
            std::fs::write(dir.join("hidden.sh"), "x").unwrap();
            std::os::unix::fs::symlink(dir.join("hidden.sh"), dir.join("notes.txt")).unwrap();
            assert!(resolve_local(Some(&dir), "notes.txt").is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn references_stay_internal() {
        for r in ["Sheet2!A1", "'My Sheet'!B2:C3", "A1:B2", "#Sheet1!A1", "Total", "", "   "] {
            assert!(matches!(classify(r), LinkKind::Internal(_)), "{r:?} -> {:?}", classify(r));
        }
    }
}
