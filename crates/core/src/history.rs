//! Clipboard history: newest first, deduplicated, capped, saved as one TOML file next to the settings.
//! Images are not in that file: each PNG (and its small thumbnail) is a file in `images/`, and the entry only points
//! at it, so the TOML stays small and the list never has to hold pixels.
use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::files::{create_private_dir, read_or_backup, write_atomic, write_private};

/// A single huge copy (a log, a base64 blob) would make every save slow and the list useless.
pub const MAX_ITEM_BYTES: usize = 1 << 20;
/// All text and HTML together. The whole list is in RAM and rewritten to disk on every copy, so 500 big entries would
/// make each copy slow (the save runs on the X11 thread, so it would also stall the hotkey); the oldest entries go first.
const MAX_TOTAL_BYTES: usize = 4 << 20;
/// All stored PNGs together (disk only: the list loads thumbnails). 500 screenshots of 8 MB would be 4 GB.
const MAX_IMAGE_TOTAL_BYTES: usize = 1 << 30;
/// Largest PNG kept. Bigger ones are skipped: 500 of them would fill the disk and a screenshot is rarely above this.
pub const MAX_IMAGE_BYTES: usize = 8 << 20;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Image {
    /// File stem of `images/<id>.png` (and `<id>.thumb.png`): a hash of the PNG, so the same image is stored once.
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u32,
    /// False when the image was too big to decode for a thumbnail; the row then shows only its label.
    #[serde(default)]
    pub thumb: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Item {
    /// Empty for an image copy.
    pub text: String,
    /// The rich flavor, when the source offered one; `text` is always the plain version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct History {
    #[serde(default)]
    items: Vec<Item>,
    /// Where the image files live.
    #[serde(skip)]
    dir: PathBuf,
    /// Bumped by every change, so a list built earlier can tell its row numbers no longer match `items()`.
    #[serde(skip)]
    rev: u64,
}

fn path() -> PathBuf {
    crate::settings::data_dir().join("history.toml")
}

fn images_dir() -> PathBuf {
    crate::settings::data_dir().join("images")
}

/// The thumbnail file of an image entry, for the UI to load.
pub fn thumb_path(id: &str) -> PathBuf {
    images_dir().join(format!("{id}.thumb.png"))
}

/// FNV-1a (64 bit) plus the length. Written out because `DefaultHasher` may change between Rust releases, which would
/// give the same image a new id after an upgrade and defeat the dedupe; this one is fixed forever.
fn id_of(png: &[u8]) -> String {
    let hash = png.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}{:x}", png.len())
}

/// Does `hay` contain `needle` (already lowercased, never empty), ignoring case? Lowercases on the fly instead of
/// keeping a lowercase copy of every entry in RAM; stops at the first mismatch, so a miss costs little per position.
fn contains_ci(hay: &str, needle: &str) -> bool {
    if needle.is_ascii() {
        // Fast path (most searches): bytes only. A multi-byte character never contains an ASCII byte, so no boundary
        // can be split. About 10x faster than the char loop below on a full history.
        let n = needle.as_bytes();
        return hay
            .as_bytes()
            .windows(n.len())
            .any(|w| w[0].to_ascii_lowercase() == n[0] && w.eq_ignore_ascii_case(n));
    }
    let needle: Vec<char> = needle.chars().collect();
    hay.char_indices().any(|(i, c)| {
        // The first character decides almost every position, so the full comparison is built only for the few hits.
        // ASCII text (most of it) skips the Unicode table lookup of `char::to_lowercase`.
        (if c.is_ascii() {
            Some(c.to_ascii_lowercase())
        } else {
            c.to_lowercase().next()
        }) == Some(needle[0])
            && {
                let mut chars = hay[i..].chars().flat_map(char::to_lowercase);
                needle.iter().all(|n| chars.next() == Some(*n))
            }
    })
}

impl History {
    pub fn load() -> Self {
        // shortcut: whole-file rewrite on every copy, fine for a few hundred items; move to SQLite beyond ~5k.
        let (mut history, sweep): (History, bool) =
            read_or_backup(&path(), |text| toml::from_str(text).map_err(|e| e.to_string()));
        history.dir = images_dir();
        // An older or hand-edited file may exceed the budget; apply it now so the list in RAM is bounded too.
        history.truncate(usize::MAX);
        // Only trust a history we actually read: with a lost or corrupt file the images may still be restorable.
        if sweep {
            history.sweep_orphans();
        }
        history
    }

    /// Deletes files in the image dir that no entry points to (history edited by hand, crash before a save).
    fn sweep_orphans(&self) {
        let used: Vec<PathBuf> = self
            .items
            .iter()
            .filter_map(|i| i.image.as_ref())
            .flat_map(|image| [self.file(&image.id, false), self.file(&image.id, true)])
            .collect();
        for entry in fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            if !used.contains(&entry.path()) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    /// An empty history whose image files go to `dir` (tests).
    pub fn with_dir(dir: PathBuf) -> Self {
        History {
            dir,
            ..Default::default()
        }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Changes whenever `items()` does (see the field).
    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// Adds a copy at the top. Returns false when it was ignored (blank or too big).
    /// The same text moves to the top instead of duplicating.
    pub fn push(&mut self, text: String, html: Option<String>, max_items: usize) -> bool {
        // A source that offers the text twice (as "html" too) would only double the file.
        let html = html.filter(|h| *h != text);
        if text.trim().is_empty()
            || text.len() > MAX_ITEM_BYTES
            || html.as_ref().is_some_and(|h| h.len() > MAX_ITEM_BYTES)
        {
            return false;
        }
        self.items.retain(|i| i.text != text);
        self.items.insert(
            0,
            Item {
                text,
                html,
                image: None,
            },
        );
        self.rev += 1;
        self.truncate(max_items);
        true
    }

    /// Adds an image copy at the top. `thumb` is a small PNG for the list, when one could be made.
    /// Returns false when it was ignored (empty, too big) or the files could not be written.
    pub fn push_image(&mut self, png: &[u8], thumb: Option<&[u8]>, size: (u32, u32), max_items: usize) -> bool {
        if png.is_empty() || png.len() > MAX_IMAGE_BYTES {
            return false;
        }
        let id = id_of(png);
        let written = create_private_dir(&self.dir)
            .and_then(|()| write_private(&self.file(&id, false), png))
            .and_then(|()| thumb.map_or(Ok(()), |t| write_private(&self.file(&id, true), t)));
        if let Err(e) = written {
            eprintln!("cannot store the image in {}: {e}", self.dir.display());
            return false;
        }
        self.items.retain(|i| i.image.as_ref().is_none_or(|im| im.id != id));
        let image = Image {
            id,
            width: size.0,
            height: size.1,
            bytes: png.len() as u32,
            thumb: thumb.is_some(),
        };
        self.items.insert(
            0,
            Item {
                text: String::new(),
                html: None,
                image: Some(image),
            },
        );
        self.rev += 1;
        self.truncate(max_items);
        true
    }

    /// The PNG bytes of an image entry.
    pub fn read_png(&self, id: &str) -> Option<Vec<u8>> {
        fs::read(self.file(id, false)).ok()
    }

    fn file(&self, id: &str, thumb: bool) -> PathBuf {
        self.dir.join(if thumb {
            format!("{id}.thumb.png")
        } else {
            format!("{id}.png")
        })
    }

    /// Keeps the newest `max` entries, and only as many as fit in `MAX_TOTAL_BYTES` of text; the dropped ones take
    /// their image files with them.
    fn truncate(&mut self, max: usize) {
        let mut keep = self.items.len().min(max);
        let (mut text, mut images) = (0, 0);
        for (i, item) in self.items.iter().enumerate().take(keep) {
            text += item.text.len() + item.html.as_ref().map_or(0, String::len);
            images += item.image.as_ref().map_or(0, |im| im.bytes as usize);
            if text > MAX_TOTAL_BYTES || images > MAX_IMAGE_TOTAL_BYTES {
                keep = i; // a single item is at most 8 MB, so the newest one always stays
                break;
            }
        }
        if self.items.len() > keep {
            let dropped = self.items.split_off(keep);
            self.delete_files(&dropped);
        }
    }

    /// Applies a lowered `max_items` right away instead of at the next copy.
    pub fn set_max(&mut self, max: usize) {
        let before = self.items.len();
        self.truncate(max);
        if self.items.len() != before {
            self.rev += 1;
        }
    }

    fn delete_files(&self, items: &[Item]) {
        for image in items.iter().filter_map(|i| i.image.as_ref()) {
            let _ = fs::remove_file(self.file(&image.id, false));
            let _ = fs::remove_file(self.file(&image.id, true));
        }
    }

    /// Indexes into `items()` whose text contains every word of `query` (case-insensitive); all when blank.
    /// An image has no text, so it only shows up while the query is blank.
    pub fn search(&self, query: &str) -> Vec<usize> {
        // Char by char, like `contains_ci` does with the text: `str::to_lowercase` turns a final Σ into ς, which would
        // never match the σ the text side produces.
        let words: Vec<String> = query
            .split_whitespace()
            .map(|w| w.chars().flat_map(char::to_lowercase).collect())
            .collect();
        let mut hits = Vec::new();
        for (idx, item) in self.items.iter().enumerate() {
            if words.iter().all(|w| contains_ci(&item.text, w)) {
                hits.push(idx);
            }
        }
        hits
    }

    pub fn move_to_top(&mut self, idx: usize) {
        if idx < self.items.len() {
            let item = self.items.remove(idx);
            self.items.insert(0, item);
            self.rev += 1;
        }
    }

    pub fn remove(&mut self, idx: usize) {
        if idx < self.items.len() {
            let item = self.items.remove(idx);
            self.delete_files(&[item]);
            self.rev += 1;
        }
    }

    pub fn clear(&mut self) {
        self.rev += 1;
        let all = std::mem::take(&mut self.items);
        self.delete_files(&all);
    }

    /// Atomic write; failures are reported on stderr and the in-memory history keeps working.
    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            eprintln!("cannot save history to {}: {e}", path().display());
        }
    }

    fn try_save(&self) -> std::io::Result<()> {
        write_atomic(&path(), toml::to_string(self).map_err(std::io::Error::other)?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn texts(h: &History) -> Vec<&str> {
        h.items().iter().map(|i| i.text.as_str()).collect()
    }

    /// A history whose image files go to a fresh temp directory.
    fn with_temp_dir() -> (History, PathBuf) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fastclipboard-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        (History::with_dir(dir.clone()), dir)
    }

    fn files(dir: &PathBuf) -> usize {
        fs::read_dir(dir).map_or(0, |d| d.count())
    }

    #[test]
    fn duplicate_moves_to_top_with_the_new_html() {
        let mut h = History::default();
        h.push("a".into(), None, 10);
        h.push("b".into(), None, 10);
        h.push("a".into(), Some("<b>a</b>".into()), 10);
        assert_eq!(texts(&h), ["a", "b"]);
        assert_eq!(h.items()[0].html.as_deref(), Some("<b>a</b>"));
    }

    #[test]
    fn html_identical_to_the_text_is_not_stored() {
        let mut h = History::default();
        h.push("plain".into(), Some("plain".into()), 10);
        assert_eq!(h.items()[0].html, None);
    }

    #[test]
    fn blank_and_oversized_are_ignored() {
        let mut h = History::default();
        assert!(!h.push("  \n".into(), None, 10));
        assert!(!h.push("x".repeat(MAX_ITEM_BYTES + 1), None, 10));
        assert!(h.items().is_empty());
    }

    #[test]
    fn cap_drops_the_oldest() {
        let mut h = History::default();
        for t in ["1", "2", "3"] {
            h.push(t.into(), None, 2);
        }
        assert_eq!(texts(&h), ["3", "2"]);
        let rev = h.rev();
        h.set_max(1);
        assert_eq!(texts(&h), ["3"]);
        assert_ne!(h.rev(), rev, "a trimmed list invalidates old row numbers");
        h.clear();
        assert!(h.items().is_empty());
    }

    #[test]
    fn search_matches_all_words_any_case() {
        let mut h = History::default();
        h.push("Hello World".into(), None, 10);
        h.push("hello there".into(), None, 10);
        assert_eq!(h.search("WORLD hello"), [1]);
        assert_eq!(h.search("").len(), 2);
        h.push("Ação É ÚTIL".into(), None, 10);
        assert_eq!(h.search("açÃO útil"), [0], "non-ASCII case folds too");
        assert!(h.search("zzz").is_empty() && h.search("worlds").is_empty());
        h.push("ΟΔΟΣ".into(), None, 10);
        assert_eq!(h.search("ΟΔΟΣ"), [0], "uppercase final sigma");
        assert_eq!(h.search("οδοσ"), [0]);
    }

    #[test]
    fn total_text_budget_drops_the_oldest_and_keeps_the_newest() {
        let mut h = History::default();
        let big = "x".repeat(MAX_ITEM_BYTES - 8); // room for the number, or push() refuses it as too big
        for i in 0..10 {
            h.push(format!("{i}{big}"), None, 500);
        }
        let total: usize = h.items().iter().map(|i| i.text.len()).sum();
        assert!(
            total <= MAX_TOTAL_BYTES && h.items().len() >= 3,
            "{} items",
            h.items().len()
        );
        assert!(h.items()[0].text.starts_with('9'), "the newest stays");
        // A history file written before the budget existed is trimmed on load.
        let mut old = History {
            items: (0..10)
                .map(|i| Item {
                    text: format!("{i}{big}"),
                    html: None,
                    image: None,
                })
                .collect(),
            ..Default::default()
        };
        old.truncate(usize::MAX);
        assert!(old.items().len() < 10);
    }

    #[test]
    fn total_image_budget_drops_the_oldest_images() {
        let image = |n: usize| Item {
            text: String::new(),
            html: None,
            image: Some(Image {
                id: format!("fake{n}"), // no files behind it: removing a missing file is ignored
                width: 1,
                height: 1,
                bytes: MAX_IMAGE_BYTES as u32,
                thumb: false,
            }),
        };
        let mut h = History {
            items: (0..200).map(image).collect(),
            ..Default::default()
        };
        h.truncate(500);
        assert_eq!(
            h.items().len(),
            MAX_IMAGE_TOTAL_BYTES / MAX_IMAGE_BYTES,
            "128 images of 8 MiB"
        );
        assert_eq!(h.items()[0].image.as_ref().unwrap().id, "fake0", "the newest stays");
    }

    #[test]
    fn image_ids_never_change() {
        // Known FNV-1a vectors: a stored id must keep matching the same image across releases.
        assert_eq!(id_of(b""), "cbf29ce4842223250");
        assert_eq!(id_of(b"a"), "af63dc4c8601ec8c1");
    }

    #[test]
    fn old_or_partial_files_load() {
        // `pinned` was written by earlier versions.
        let h: History = toml::from_str("[[items]]\ntext = \"x\"\npinned = true").unwrap();
        assert_eq!(texts(&h), ["x"]);
        assert!(toml::from_str::<History>("").unwrap().items().is_empty());
    }

    #[test]
    fn images_are_files_deduplicated_and_removed_with_their_entry() {
        let (mut h, dir) = with_temp_dir();
        assert!(h.push_image(b"png-one", Some(b"thumb-one"), (4, 3), 10));
        assert!(
            h.push_image(b"png-one", Some(b"thumb-one"), (4, 3), 10),
            "same image again"
        );
        assert_eq!(h.items().len(), 1);
        assert_eq!(files(&dir), 2, "png + thumbnail, stored once");
        let id = h.items()[0].image.as_ref().unwrap().id.clone();
        assert_eq!(h.read_png(&id).as_deref(), Some(&b"png-one"[..]));
        assert!(h.search("").contains(&0) && h.search("anything").is_empty());

        h.push_image(b"png-two", None, (1, 1), 10);
        assert!(!h.items()[0].image.as_ref().unwrap().thumb);
        h.remove(1); // png-one
        assert_eq!(files(&dir), 1);
        h.clear();
        assert_eq!(files(&dir), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sweep_removes_only_unreferenced_files() {
        let (mut h, dir) = with_temp_dir();
        h.push_image(b"kept", Some(b"kept-thumb"), (1, 1), 10);
        fs::write(dir.join("deadbeef.png"), b"x").unwrap();
        fs::write(dir.join("deadbeef.thumb.png"), b"x").unwrap();
        assert_eq!(files(&dir), 4);
        h.sweep_orphans();
        assert_eq!(files(&dir), 2, "the entry's png + thumbnail stay");
        let id = h.items()[0].image.as_ref().unwrap().id.clone();
        assert!(h.read_png(&id).is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_cap_deletes_the_files_of_dropped_images() {
        let (mut h, dir) = with_temp_dir();
        h.push_image(b"first", None, (1, 1), 2);
        h.push("text".into(), None, 2);
        h.push("more".into(), None, 2);
        assert_eq!(texts(&h), ["more", "text"]);
        assert_eq!(files(&dir), 0);
        assert!(!h.push_image(&vec![0; MAX_IMAGE_BYTES + 1], None, (1, 1), 2), "too big");
        assert!(!h.push_image(b"", None, (1, 1), 2), "empty");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rev_changes_with_every_edit_so_stale_row_numbers_are_detectable() {
        let mut h = History::default();
        let mut last = h.rev();
        let mut changed = |h: &History| {
            assert_ne!(h.rev(), last);
            last = h.rev();
        };
        h.push("a".into(), None, 10);
        changed(&h);
        h.push("b".into(), None, 10);
        changed(&h);
        h.move_to_top(1);
        changed(&h);
        h.remove(0);
        changed(&h);
        h.clear();
        changed(&h);
        let (mut img, dir) = with_temp_dir();
        img.push_image(b"png", None, (1, 1), 10);
        assert_ne!(img.rev(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn move_to_top_reorders() {
        let mut h = History::default();
        for t in ["a", "b", "c"] {
            h.push(t.into(), None, 10);
        }
        h.move_to_top(2); // "a"
        assert_eq!(texts(&h), ["a", "c", "b"]);
    }
}
