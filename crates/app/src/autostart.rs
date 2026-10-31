//! "Start at login" as an XDG autostart entry, toggled from the preferences.
use std::{fs, io, path::PathBuf};

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("autostart/fastpaste.desktop")
}

pub fn is_enabled() -> bool {
    path().exists()
}

/// The AppImage file when running from one (its mount point disappears on exit), else this binary.
fn exe() -> Option<String> {
    std::env::var("APPIMAGE")
        .ok()
        .or_else(|| std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()))
}

/// An `Exec` argument as the Desktop Entry spec wants it: double-quoted, with `"`, `` ` ``, `$` and `\` escaped,
/// the backslashes doubled again for the file's own string escaping, and `%` doubled (it starts a field code).
fn quote(arg: &str) -> String {
    let mut s = String::from("\"");
    for c in arg.chars() {
        match c {
            '"' | '`' | '$' => {
                s.push_str("\\\\");
                s.push(c);
            }
            '\\' => s.push_str("\\\\\\\\"),
            '%' => s.push_str("%%"),
            _ => s.push(c),
        }
    }
    s.push('"');
    s
}

fn entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=FastPaste\nComment=Clipboard history with a global shortcut\n\
         Exec={}\nIcon=io.github.evandro777.FastPaste\nTerminal=false\n",
        quote(exe)
    )
}

pub fn set(enabled: bool) -> io::Result<()> {
    let p = path();
    if enabled {
        let exe = exe().ok_or_else(|| io::Error::other("cannot find the executable"))?;
        fs::create_dir_all(p.parent().unwrap_or(&p))?;
        fs::write(&p, entry(&exe))
    } else {
        match fs::remove_file(&p) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_path_is_quoted_for_spaces_and_reserved_characters() {
        assert_eq!(quote("/home/a b, c/fastpaste"), "\"/home/a b, c/fastpaste\"");
        assert_eq!(quote("/x/$y\"z%"), "\"/x/\\\\$y\\\\\"z%%\"");
        assert!(entry("/bin/fastpaste").contains("\nExec=\"/bin/fastpaste\"\n"));
    }
}
