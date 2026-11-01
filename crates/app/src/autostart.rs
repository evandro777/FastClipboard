//! "Start at login" as an XDG autostart entry, toggled from the preferences.
use std::{collections::HashMap, fs, io, path::PathBuf};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

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

pub fn in_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
}

/// The `RequestBackground` options. `commandline` is what the host runs at login (the sandbox adds `flatpak run`).
fn portal_options(enabled: bool, token: &str) -> HashMap<&'static str, Value<'static>> {
    HashMap::from([
        ("handle_token", Value::from(token.to_string())),
        (
            "reason",
            Value::from("Open the clipboard history with its shortcut after login".to_string()),
        ),
        ("autostart", Value::from(enabled)),
        ("commandline", Value::from(vec!["fastpaste".to_string()])),
    ])
}

/// Inside Flatpak the real `~/.config/autostart` is out of reach: the Background portal writes the entry (and shows
/// the desktop's permission dialog, so this blocks: call it from a worker thread).
fn portal(enabled: bool) -> Result<(), String> {
    let err = |e: zbus::Error| e.to_string();
    let conn = Connection::session().map_err(err)?;
    let token = format!("fastpaste{}", std::process::id());
    let sender = conn
        .unique_name()
        .map_or_else(String::new, |n| n.trim_start_matches(':').replace('.', "_"));
    // Subscribe before the call: a fast answer would otherwise be lost.
    let request = Proxy::new(
        &conn,
        "org.freedesktop.portal.Desktop",
        format!("/org/freedesktop/portal/desktop/request/{sender}/{token}"),
        "org.freedesktop.portal.Request",
    )
    .map_err(err)?;
    let mut responses = request.receive_signal("Response").map_err(err)?;
    Proxy::new(
        &conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Background",
    )
    .map_err(err)?
    .call::<_, _, OwnedObjectPath>("RequestBackground", &("", portal_options(enabled, &token)))
    .map_err(err)?;
    let msg = responses.next().ok_or("the portal sent no answer")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize().map_err(|e| e.to_string())?;
    let granted = results
        .get("autostart")
        .is_none_or(|v| bool::try_from(v).unwrap_or(enabled) == enabled);
    match (code, granted) {
        (0, true) => Ok(()),
        (0, false) | (1, _) => {
            Err("the desktop did not allow it (reset with: flatpak permission-reset $FLATPAK_ID)".into())
        }
        _ => Err(format!("the portal answered {code}")),
    }
}

/// shortcut: in Flatpak the marker file below only records the choice for `is_enabled`; if the user removes the entry in
/// the desktop's Startup Applications the switch stays on. Fine until the portal offers a way to query it.
pub fn set(enabled: bool) -> io::Result<()> {
    if in_flatpak() {
        portal(enabled).map_err(io::Error::other)?;
    }
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

    #[test]
    fn portal_options_carry_the_choice_and_the_command() {
        let on = portal_options(true, "tok");
        assert_eq!(on["autostart"], Value::from(true));
        assert_eq!(on["handle_token"], Value::from("tok".to_string()));
        assert_eq!(on["commandline"], Value::from(vec!["fastpaste".to_string()]));
        assert_eq!(portal_options(false, "t")["autostart"], Value::from(false));
    }
}
