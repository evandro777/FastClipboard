//! Text from an image through the `tesseract` command (optional, found on PATH). Shelling out keeps the C libraries and
//! the language models out of this app; the process only exists during the call.
use std::{
    io::Write,
    process::{Command, Stdio},
};

/// `-l` takes language codes joined with `+` ("eng+por"). Anything else would be read as another tesseract option.
fn valid_lang(lang: &str) -> bool {
    !lang.is_empty()
        && lang
            .split('+')
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

/// The sandbox cannot see the host's tesseract; `flatpak-spawn --host` reaches it, but only once the user has granted
/// `org.freedesktop.Flatpak` (see the README), so that is a deliberate opt-in.
fn tesseract() -> Command {
    if crate::autostart::in_flatpak() {
        let mut c = Command::new("flatpak-spawn");
        c.args(["--host", "tesseract"]);
        c
    } else {
        Command::new("tesseract")
    }
}

/// Language packs `tesseract --list-langs` reports (empty when tesseract cannot run).
fn installed() -> Vec<String> {
    tesseract()
        .arg("--list-langs")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.is_empty() && !l.contains(' '))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The pack for the UI language (pt_BR -> por) first, then `eng`, keeping only packs that are installed so a missing
/// one does not make tesseract fail. shortcut: five languages mapped by hand; use `ocr_lang` for anything else.
fn pick_langs(ui_lang: &str, installed: &[String]) -> String {
    let extra = match ui_lang.split(['_', '-']).next().unwrap_or("") {
        "pt" => "por",
        "es" => "spa",
        "fr" => "fra",
        "de" => "deu",
        "it" => "ita",
        _ => "",
    };
    let picked: Vec<_> = [extra, "eng"]
        .into_iter()
        .filter(|l| !l.is_empty() && installed.iter().any(|i| i == l))
        .collect();
    if picked.is_empty() {
        "eng".into()
    } else {
        picked.join("+")
    }
}

/// The languages for the interface language `ui_lang` (see `pick_langs`). Runs tesseract: call from a worker thread.
pub fn auto_lang(ui_lang: &str) -> String {
    pick_langs(ui_lang, &installed())
}

/// Blocks for a second or two: call it from a worker thread.
pub fn extract(png: &[u8], lang: &str) -> Result<String, String> {
    if !valid_lang(lang) {
        return Err(format!("ocr_lang \"{lang}\" is not like \"eng+por\""));
    }
    let mut child = tesseract()
        .args(["stdin", "stdout", "-l", lang])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run tesseract (is it installed?): {e}"))?;
    // tesseract reads the whole image before it prints anything, so writing first cannot deadlock.
    let written = child.stdin.take().map(|mut stdin| stdin.write_all(png));
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        let hint = if crate::autostart::in_flatpak() {
            " (in Flatpak OCR needs: flatpak override --user --talk-name=org.freedesktop.Flatpak $FLATPAK_ID, and tesseract on the host)"
        } else {
            ""
        };
        return Err(format!("tesseract failed: {}{hint}", why.trim()));
    }
    if let Some(Err(e)) = written {
        return Err(format!("cannot send the image to tesseract: {e}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::{pick_langs, valid_lang};

    #[test]
    fn languages_follow_the_ui_language_when_installed() {
        let both = ["eng".to_string(), "osd".into(), "por".into()];
        assert_eq!(pick_langs("pt_BR", &both), "por+eng");
        assert_eq!(pick_langs("en_US", &both), "eng");
        assert_eq!(pick_langs("", &both), "eng");
        assert_eq!(pick_langs("de_DE", &both), "eng", "no German pack: do not ask for it");
        assert_eq!(pick_langs("pt", &["eng".to_string()]), "eng");
        assert_eq!(pick_langs("pt", &[]), "eng");
    }

    #[test]
    fn language_codes() {
        assert!(valid_lang("eng") && valid_lang("eng+por") && valid_lang("chi_sim"));
        assert!(!valid_lang("") && !valid_lang("eng+") && !valid_lang("-c tessedit") && !valid_lang("eng por"));
    }
}
