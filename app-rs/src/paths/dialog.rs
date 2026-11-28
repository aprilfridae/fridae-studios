//! The system's own file and folder dialogs.
//!
//! Windows and macOS: rfd (the system dialog in the app's process). Linux: rfd needs either GTK 3 bindings of another
//! version than wry's or its portal feature (the `pollster` crate), neither of which this build carries, so there the
//! dialog is the desktop's own dialog program: `zenity` (GNOME and most desktops) or else `kdialog` (KDE). When
//! neither is installed the picker says so in the log and returns nothing (the window's typed path fields still work).

use std::path::PathBuf;

/// What the dialog asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Folder,
    File,
    Files,
    Save,
}

/// A dialog to show: built like rfd's `FileDialog`, shown by one of the `pick_*` / `save_file` calls.
#[derive(Clone, Debug, Default)]
pub struct Dialog {
    title: String,
    directory: Option<PathBuf>,
    file_name: Option<String>,
    filters: Vec<(String, Vec<String>)>,
}

impl Dialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    pub fn directory(mut self, dir: impl Into<PathBuf>) -> Self {
        self.directory = Some(dir.into());
        self
    }

    pub fn file_name(mut self, name: &str) -> Self {
        self.file_name = Some(name.to_string());
        self
    }

    pub fn filter(mut self, name: &str, extensions: &[&str]) -> Self {
        self.filters.push((name.to_string(), extensions.iter().map(|e| e.to_string()).collect()));
        self
    }

    pub fn pick_folder(self) -> Option<PathBuf> {
        self.show(Kind::Folder).into_iter().next()
    }

    pub fn pick_file(self) -> Option<PathBuf> {
        self.show(Kind::File).into_iter().next()
    }

    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        Some(self.show(Kind::Files)).filter(|v| !v.is_empty())
    }

    pub fn save_file(self) -> Option<PathBuf> {
        self.show(Kind::Save).into_iter().next()
    }

    #[cfg(not(target_os = "linux"))]
    fn show(self, kind: Kind) -> Vec<PathBuf> {
        let mut d = rfd::FileDialog::new();
        if !self.title.is_empty() {
            d = d.set_title(&self.title);
        }
        if let Some(dir) = &self.directory {
            d = d.set_directory(dir);
        }
        if let Some(name) = &self.file_name {
            d = d.set_file_name(name);
        }
        for (name, exts) in &self.filters {
            let exts: Vec<&str> = exts.iter().map(String::as_str).collect();
            d = d.add_filter(name, &exts);
        }
        match kind {
            Kind::Folder => d.pick_folder().into_iter().collect(),
            Kind::File => d.pick_file().into_iter().collect(),
            Kind::Files => d.pick_files().unwrap_or_default(),
            Kind::Save => d.save_file().into_iter().collect(),
        }
    }

    #[cfg(target_os = "linux")]
    fn show(self, kind: Kind) -> Vec<PathBuf> {
        use crate::paths::Start;
        let programs: [(&str, Vec<String>); 2] = [("zenity", zenity_args(&self, kind)), ("kdialog", kdialog_args(&self, kind))];
        for (name, args) in programs {
            let Some(program) = crate::paths::which(name) else { continue };
            let out = std::process::Command::new(program).args(&args).stdin(std::process::Stdio::null()).run_output();
            return match out {
                // 0: chosen; anything else (1 for both programs) is the user cancelling
                Ok(o) if o.status.success() => parse_output(&String::from_utf8_lossy(&o.stdout)),
                _ => Vec::new(),
            };
        }
        eprintln!("Fridae: no file dialog program on this computer (install zenity or kdialog)");
        Vec::new()
    }
}

/// `zenity --file-selection ...` for a dialog.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub fn zenity_args(d: &Dialog, kind: Kind) -> Vec<String> {
    let mut a = vec!["--file-selection".to_string()];
    if !d.title.is_empty() {
        a.push(format!("--title={}", d.title));
    }
    match kind {
        Kind::Folder => a.push("--directory".into()),
        Kind::Files => a.extend(["--multiple".to_string(), "--separator=\n".to_string()]),
        Kind::Save => a.extend(["--save".to_string(), "--confirm-overwrite".to_string()]),
        Kind::File => {}
    }
    // a folder to start in ends with "/", so zenity opens it rather than selecting it
    match (&d.directory, &d.file_name, kind) {
        (Some(dir), Some(name), Kind::Save) => a.push(format!("--filename={}", unix_join(dir, name))),
        (None, Some(name), Kind::Save) => a.push(format!("--filename={name}")),
        (Some(dir), _, _) => a.push(format!("--filename={}/", dir.display().to_string().trim_end_matches('/'))),
        _ => {}
    }
    if kind != Kind::Folder {
        for (name, exts) in &d.filters {
            a.push(format!("--file-filter={name} | {}", globs(exts)));
        }
    }
    a
}

/// `kdialog --get...` for a dialog.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub fn kdialog_args(d: &Dialog, kind: Kind) -> Vec<String> {
    let mut a = Vec::new();
    if !d.title.is_empty() {
        a.extend(["--title".to_string(), d.title.clone()]);
    }
    let start = match (&d.directory, &d.file_name, kind) {
        (Some(dir), Some(name), Kind::Save) => unix_join(dir, name),
        (None, Some(name), Kind::Save) => name.clone(),
        (Some(dir), _, _) => dir.display().to_string(),
        _ => ".".to_string(),
    };
    match kind {
        Kind::Folder => {
            a.extend(["--getexistingdirectory".to_string(), start]);
            return a;
        }
        Kind::Files => a.extend(["--multiple".to_string(), "--separate-output".to_string(), "--getopenfilename".to_string(), start]),
        Kind::File => a.extend(["--getopenfilename".to_string(), start]),
        Kind::Save => a.extend(["--getsavefilename".to_string(), start]),
    }
    let filter: Vec<String> = d.filters.iter().map(|(name, exts)| format!("{name} ({})", globs(exts))).collect();
    if !filter.is_empty() {
        a.push(filter.join("\n"));
    }
    a
}

/// A folder and a file name joined with "/" (these programs are Linux's; tests run everywhere).
fn unix_join(dir: &std::path::Path, name: &str) -> String {
    format!("{}/{name}", dir.display().to_string().trim_end_matches('/'))
}

fn globs(exts: &[String]) -> String {
    exts.iter().map(|e| if e == "*" || e.is_empty() { "*".to_string() } else { format!("*.{e}") }).collect::<Vec<_>>().join(" ")
}

/// The paths a dialog program printed, one per line.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub fn parse_output(out: &str) -> Vec<PathBuf> {
    out.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video() -> Dialog {
        Dialog::new().title("Add clips").filter("Video", &["mp4", "mov"])
    }

    #[test]
    fn zenity_is_asked_for_the_right_kind_of_dialog() {
        assert_eq!(zenity_args(&Dialog::new().title("Choose the project's folder"), Kind::Folder), ["--file-selection", "--title=Choose the project's folder", "--directory"]);
        assert_eq!(zenity_args(&video(), Kind::Files), ["--file-selection", "--title=Add clips", "--multiple", "--separator=\n", "--file-filter=Video | *.mp4 *.mov"]);
        let save = Dialog::new().title("Save the promo").directory("/home/a/Downloads").file_name("promo.mp4").filter("MP4 video", &["mp4"]);
        assert_eq!(zenity_args(&save, Kind::Save), ["--file-selection", "--title=Save the promo", "--save", "--confirm-overwrite", "--filename=/home/a/Downloads/promo.mp4", "--file-filter=MP4 video | *.mp4"]);
        assert_eq!(zenity_args(&Dialog::new().directory("/home/a/"), Kind::File), ["--file-selection", "--filename=/home/a/"], "a folder to start in ends with one slash");
    }

    #[test]
    fn kdialog_is_asked_for_the_right_kind_of_dialog() {
        assert_eq!(kdialog_args(&Dialog::new().title("Pick"), Kind::Folder), ["--title", "Pick", "--getexistingdirectory", "."]);
        assert_eq!(kdialog_args(&video(), Kind::Files), ["--title", "Add clips", "--multiple", "--separate-output", "--getopenfilename", ".", "Video (*.mp4 *.mov)"]);
        let save = Dialog::new().directory("/home/a/Downloads").file_name("promo.mp4").filter("MP4 video", &["mp4"]);
        assert_eq!(kdialog_args(&save, Kind::Save), ["--getsavefilename", "/home/a/Downloads/promo.mp4", "MP4 video (*.mp4)"]);
    }

    #[test]
    fn a_dialog_programs_output_is_one_path_per_line() {
        assert_eq!(parse_output("/home/a/one.mp4\n/home/a/two words.mov\n"), [PathBuf::from("/home/a/one.mp4"), PathBuf::from("/home/a/two words.mov")]);
        assert!(parse_output("\n").is_empty());
    }
}
