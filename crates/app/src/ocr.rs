//! Text from an image through the `tesseract` command (optional, found on PATH). Shelling out keeps the C libraries and
//! the language models out of this app; the process only exists during the call.
use std::{
    io::{self, Read, Write},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

/// A hung tesseract (or a host call waiting on a portal) must not block the worker, or `ocr_checking`, forever.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Reads a pipe to its end on a thread, so a full pipe never stalls the child.
fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
}

/// Runs `cmd` with `input` on stdin and kills it after `timeout`. Also returns the result of writing the input.
/// Polls `try_wait` (std has no wait with a timeout); the 50 ms step is nothing next to tesseract's seconds.
fn run(cmd: &mut Command, input: &[u8], timeout: Duration) -> io::Result<(Output, io::Result<()>)> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take();
    let input = input.to_vec();
    // tesseract reads the whole image before it prints anything, but the pipes are drained on threads anyway.
    let writer = thread::spawn(move || stdin.map_or(Ok(()), |mut s| s.write_all(&input)));
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > timeout {
            // The reader threads are left to end on their own: a grandchild may still hold the pipes.
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "timed out"));
        }
        thread::sleep(Duration::from_millis(50));
    };
    let written = writer
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("writer thread panicked")));
    let output = Output {
        status,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    };
    Ok((output, written))
}

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

/// Language packs `tesseract --list-langs` reports; `None` when tesseract cannot run (not found, or no permission).
fn installed() -> Option<Vec<String>> {
    let (out, _) = run(tesseract().arg("--list-langs"), &[], TIMEOUT)
        .ok()
        .filter(|(o, _)| o.status.success())?;
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.is_empty() && !l.contains(' '))
            .map(str::to_string)
            .collect(),
    )
}

/// Whether OCR can run, checked before the menu offers it.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Ready,
    /// Flatpak without `org.freedesktop.Flatpak`: the host's tesseract cannot be started.
    NoPermission,
    NoTesseract,
    /// Language packs that are wanted (by the setting, or `eng` in automatic mode) but not installed.
    MissingLang(Vec<String>),
}

impl Status {
    /// The `ocr-state` of the preferences window (0 ready, 1 no permission, 2 no tesseract, 3 missing languages).
    pub fn code(&self) -> i32 {
        match self {
            Status::Ready => 0,
            Status::NoPermission => 1,
            Status::NoTesseract => 2,
            Status::MissingLang(_) => 3,
        }
    }

    /// The missing codes, for the message.
    pub fn missing(&self) -> String {
        match self {
            Status::MissingLang(l) => l.join(", "),
            _ => String::new(),
        }
    }

    /// What to run to fix it; empty when the help page is enough (package names depend on the distribution).
    pub fn command(&self) -> String {
        match self {
            Status::NoPermission => {
                "flatpak override --user --talk-name=org.freedesktop.Flatpak io.github.evandro777.FastClipboard".into()
            }
            _ => String::new(),
        }
    }
}

/// Checks tesseract and the wanted language packs (`ocr_lang` as in the settings, "" = automatic). Runs processes:
/// call from a worker thread.
pub fn status(ocr_lang: &str, ui_lang: &str) -> Status {
    let Some(langs) = installed() else {
        // Without the permission flatpak-spawn fails at once, just as it does for a missing host tesseract; a command
        // that always exists tells the two apart.
        let host_ok = !crate::autostart::in_flatpak()
            || run(Command::new("flatpak-spawn").args(["--host", "true"]), &[], TIMEOUT)
                .is_ok_and(|(o, _)| o.status.success());
        return if host_ok {
            Status::NoTesseract
        } else {
            Status::NoPermission
        };
    };
    classify(&langs, ocr_lang, ui_lang)
}

fn classify(installed: &[String], ocr_lang: &str, ui_lang: &str) -> Status {
    let missing: Vec<String> = if ocr_lang.is_empty() {
        pick_langs(ui_lang, installed).map_or_else(|| vec!["eng".into()], |_| vec![])
    } else if !valid_lang(ocr_lang) {
        vec![ocr_lang.to_string()]
    } else {
        ocr_lang
            .split('+')
            .filter(|l| !installed.iter().any(|i| i == l))
            .map(str::to_string)
            .collect()
    };
    if missing.is_empty() {
        Status::Ready
    } else {
        Status::MissingLang(missing)
    }
}

/// The pack for the UI language (pt_BR -> por) first, then `eng`, keeping only packs that are installed so a missing
/// one does not make tesseract fail; `None` when neither is installed.
/// shortcut: five languages mapped by hand; use `ocr_lang` for anything else.
fn pick_langs(ui_lang: &str, installed: &[String]) -> Option<String> {
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
    (!picked.is_empty()).then(|| picked.join("+"))
}

/// The languages for the interface language `ui_lang` (see `pick_langs`). Runs tesseract: call from a worker thread.
/// With nothing installed it still answers `eng`, so tesseract's own error reaches stderr.
pub fn auto_lang(ui_lang: &str) -> String {
    pick_langs(ui_lang, &installed().unwrap_or_default()).unwrap_or_else(|| "eng".into())
}

/// Blocks for a second or two: call it from a worker thread.
pub fn extract(png: &[u8], lang: &str) -> Result<String, String> {
    if !valid_lang(lang) {
        return Err(format!("ocr_lang \"{lang}\" is not like \"eng+por\""));
    }
    let (out, written) = run(tesseract().args(["stdin", "stdout", "-l", lang]), png, TIMEOUT)
        .map_err(|e| format!("cannot run tesseract (is it installed?): {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        let hint = if crate::autostart::in_flatpak() {
            " (in Flatpak OCR needs: flatpak override --user --talk-name=org.freedesktop.Flatpak $FLATPAK_ID, and tesseract on the host)"
        } else {
            ""
        };
        return Err(format!("tesseract failed: {}{hint}", why.trim()));
    }
    if let Err(e) = written {
        return Err(format!("cannot send the image to tesseract: {e}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::{Status, classify, pick_langs, run, valid_lang};
    use std::{io::ErrorKind, process::Command, time::Duration};

    #[test]
    fn run_feeds_stdin_and_collects_the_output() {
        let (out, written) = run(&mut Command::new("cat"), b"hello", Duration::from_secs(10)).unwrap();
        assert!(out.status.success() && written.is_ok());
        assert_eq!(out.stdout, b"hello");
    }

    #[test]
    fn run_kills_a_hung_process() {
        let started = std::time::Instant::now();
        let err = run(Command::new("sleep").arg("5"), &[], Duration::from_millis(100)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "returned at the timeout, not at the exit"
        );
    }

    #[test]
    fn languages_follow_the_ui_language_when_installed() {
        let both = ["eng".to_string(), "osd".into(), "por".into()];
        assert_eq!(pick_langs("pt_BR", &both).as_deref(), Some("por+eng"));
        assert_eq!(pick_langs("en_US", &both).as_deref(), Some("eng"));
        assert_eq!(pick_langs("", &both).as_deref(), Some("eng"));
        assert_eq!(
            pick_langs("de_DE", &both).as_deref(),
            Some("eng"),
            "no German pack: do not ask for it"
        );
        assert_eq!(pick_langs("pt", &["eng".to_string()]).as_deref(), Some("eng"));
        assert_eq!(pick_langs("pt", &[]), None);
    }

    #[test]
    fn status_from_the_installed_packs() {
        let eng = ["eng".to_string(), "osd".into()];
        assert_eq!(classify(&eng, "", "pt_BR"), Status::Ready, "auto: eng is enough");
        assert_eq!(classify(&[], "", "en"), Status::MissingLang(vec!["eng".into()]));
        assert_eq!(classify(&eng, "eng+por", "en"), Status::MissingLang(vec!["por".into()]));
        assert_eq!(classify(&eng, "eng", "en"), Status::Ready);
        assert_eq!(
            classify(&eng, "-c x", "en").code(),
            3,
            "a malformed ocr_lang is reported, not run"
        );
        assert!(Status::NoPermission.command().starts_with("flatpak override"));
        assert_eq!(Status::NoTesseract.command(), "");
    }

    #[test]
    fn language_codes() {
        assert!(valid_lang("eng") && valid_lang("eng+por") && valid_lang("chi_sim"));
        assert!(!valid_lang("") && !valid_lang("eng+") && !valid_lang("-c tessedit") && !valid_lang("eng por"));
    }
}
