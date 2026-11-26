//! "Start with the system" (since 0.4.3): the entry that makes the system start Aria at login,
//! with [`AUTOSTART_ARG`], for the current user only.
//!
//! - Windows: the value `Aria` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//! - macOS: `~/Library/LaunchAgents/io.github.hixenov.aria.plist`.
//! - Linux and the BSDs: `$XDG_CONFIG_HOME/autostart/io.github.hixenov.aria.desktop`.
//!
//! [`Entry::apply`] writes the entry when it is missing or different, and removes it only when it
//! is Aria's own (its command has [`AUTOSTART_ARG`]); nothing else is ever touched.

use std::io;
use std::path::{Path, PathBuf};

pub use aria_core::opened::AUTOSTART_ARG;

/// Applies the setting: `true` registers Aria, `false` removes its entry. An error says why it
/// could not, in words for the log.
pub type Autostart = Box<dyn FnMut(bool) -> Result<(), String>>;

/// One system's entry for Aria.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A file Aria owns, written whole.
    File { path: PathBuf, content: String },
    /// A string value under `HKEY_CURRENT_USER\<subkey>`.
    #[cfg(windows)]
    Registry {
        subkey: String,
        name: String,
        data: String,
    },
}

impl Entry {
    /// Writes the entry (`on`) when it is missing or different, or removes it (`!on`) if it is
    /// Aria's own: its command has [`AUTOSTART_ARG`]. Anything else under that name is left as
    /// it is.
    pub fn apply(&self, on: bool) -> io::Result<()> {
        match self {
            Entry::File { path, content } => apply_file(path, content, on),
            #[cfg(windows)]
            Entry::Registry { subkey, name, data } => apply_registry(subkey, name, data, on),
        }
    }
}

fn apply_file(path: &Path, content: &str, on: bool) -> io::Result<()> {
    let current = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        // A file that is not text is not Aria's.
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return if on { Err(error) } else { Ok(()) };
        }
        Err(error) => return Err(error),
    };
    if on {
        if content.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Aria's path cannot be written in a startup entry",
            ));
        }
        if current.as_deref() == Some(content) {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)
    } else if current.is_some_and(|text| text.contains(AUTOSTART_ARG)) {
        std::fs::remove_file(path)
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn apply_registry(subkey: &str, name: &str, data: &str, on: bool) -> io::Result<()> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    let user = RegKey::predef(HKEY_CURRENT_USER);
    if on {
        let (key, _) = user.create_subkey(subkey)?;
        if key.get_value::<String, _>(name).ok().as_deref() == Some(data) {
            return Ok(());
        }
        return key.set_value(name, &data.to_owned());
    }
    let key = match user.open_subkey_with_flags(subkey, KEY_READ | KEY_WRITE) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    match key.get_value::<String, _>(name) {
        Ok(current) if current.contains(AUTOSTART_ARG) => key.delete_value(name),
        _ => Ok(()),
    }
}

/// The name Aria's startup entry goes by: the macOS bundle identifier and the Linux file name.
const ID: &str = "io.github.hixenov.aria";
/// Where Windows keeps the current user's programs started at login.
#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// The Windows `Run` value: the quoted executable and [`AUTOSTART_ARG`].
pub fn run_value(exe: &Path) -> String {
    format!("\"{}\" {AUTOSTART_ARG}", exe.display())
}

/// The Linux autostart file for `exe`, or `None` when its path cannot be written there (not
/// text, or a line break in it).
pub fn desktop_entry(exe: &Path) -> Option<String> {
    let path = exe.to_str()?;
    if path.contains(['\n', '\r']) {
        return None;
    }
    // The desktop entry spec: inside a quoted argument `"`, `` ` ``, `$` and `\` take a
    // backslash; then, as for any string value, every backslash is doubled; and `%` is `%%`.
    let mut quoted = String::new();
    for c in path.chars() {
        match c {
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(c);
            }
            '\\' => quoted.push_str("\\\\\\\\"),
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    Some(format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Aria\n\
         Comment=Starts Aria in the tray when you log in\n\
         Exec=\"{quoted}\" {AUTOSTART_ARG}\n\
         Icon=aria\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    ))
}

/// The macOS launch agent for `exe`, or `None` when its path is not text.
pub fn launch_agent(exe: &Path) -> Option<String> {
    let path = xml_text(exe.to_str()?);
    Some(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n    \
         <key>Label</key>\n    \
         <string>{ID}</string>\n    \
         <key>ProgramArguments</key>\n    \
         <array>\n        \
         <string>{path}</string>\n        \
         <string>{AUTOSTART_ARG}</string>\n    \
         </array>\n    \
         <key>RunAtLoad</key>\n    \
         <true/>\n\
         </dict>\n\
         </plist>\n"
    ))
}

/// `text` safe inside an XML element.
fn xml_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// This system's entry for `exe`.
pub fn system_entry(exe: &Path) -> Result<Entry, String> {
    #[cfg(windows)]
    {
        Ok(Entry::Registry {
            subkey: RUN_KEY.to_owned(),
            name: "Aria".to_owned(),
            data: run_value(exe),
        })
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or("no home folder")?;
        Ok(Entry::File {
            path: PathBuf::from(home)
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{ID}.plist")),
            content: launch_agent(exe).unwrap_or_default(),
        })
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or("no config folder")?;
        Ok(Entry::File {
            path: config.join("autostart").join(format!("{ID}.desktop")),
            content: desktop_entry(exe).unwrap_or_default(),
        })
    }
}

/// Applies the setting for the running executable.
pub fn system() -> Autostart {
    Box::new(|on| {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let entry = system_entry(&exe)?;
        entry.apply(on).map_err(|error| error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_value_quotes_the_path() {
        assert_eq!(
            run_value(Path::new(r"C:\Program Files\Aria\aria.exe")),
            r#""C:\Program Files\Aria\aria.exe" --autostart"#
        );
    }

    #[test]
    fn the_desktop_entry_quotes_the_path() {
        let entry =
            desktop_entry(Path::new("/home/a b/\"q\"/$x/`y`/50%/back\\slash/aria")).unwrap();
        assert!(entry.starts_with("[Desktop Entry]\n"), "{entry}");
        // Quoting rule, then the string escape (a backslash doubled), then %% for %.
        let exec = r#"Exec="/home/a b/\\"q\\"/\\$x/\\`y\\`/50%%/back\\\\slash/aria" --autostart"#;
        assert!(entry.lines().any(|line| line == exec), "{entry}");
        assert!(entry.lines().any(|line| line == "Type=Application"));
        assert!(entry.lines().any(|line| line == "Name=Aria"));
        assert!(desktop_entry(Path::new("/home/new\nline/aria")).is_none());
    }

    #[test]
    fn the_launch_agent_escapes_xml() {
        let agent =
            launch_agent(Path::new("/Applications/A&B <x>.app/Contents/MacOS/aria")).unwrap();
        assert!(agent.contains("<string>io.github.hixenov.aria</string>"));
        assert!(
            agent.contains(
                "<string>/Applications/A&amp;B &lt;x&gt;.app/Contents/MacOS/aria</string>"
            )
        );
        assert!(agent.contains("<string>--autostart</string>"));
        assert!(agent.contains("<key>RunAtLoad</key>\n    <true/>"));
    }

    #[test]
    fn a_file_entry_is_written_rewritten_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("autostart")
            .join("io.github.hixenov.aria.desktop");
        let entry = |content: &str| Entry::File {
            path: path.clone(),
            content: content.to_owned(),
        };
        entry("Exec=old --autostart\n").apply(true).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Exec=old --autostart\n"
        );
        entry("Exec=new --autostart\n").apply(true).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Exec=new --autostart\n"
        );
        entry("Exec=new --autostart\n").apply(false).unwrap();
        assert!(!path.exists());
        // Removing what is not there is fine.
        entry("Exec=new --autostart\n").apply(false).unwrap();
    }

    #[test]
    fn a_foreign_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("io.github.hixenov.aria.desktop");
        std::fs::write(&path, "Exec=someone-else\n").unwrap();
        let entry = Entry::File {
            path: path.clone(),
            content: "Exec=aria --autostart\n".to_owned(),
        };
        entry.apply(false).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Exec=someone-else\n"
        );
    }

    #[test]
    fn an_empty_entry_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.desktop");
        let entry = Entry::File {
            path: path.clone(),
            content: String::new(),
        };
        assert!(entry.apply(true).is_err());
        assert!(!path.exists());
    }

    /// A throwaway key under `HKCU\Software`, deleted when dropped.
    #[cfg(windows)]
    struct TestKey(String);

    #[cfg(windows)]
    impl TestKey {
        fn new() -> TestKey {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            TestKey(format!(r"Software\AriaTest-{}-{nanos}", std::process::id()))
        }

        fn run(&self) -> String {
            format!(r"{}\Run", self.0)
        }

        fn value(&self) -> Option<String> {
            let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
                .open_subkey(self.run())
                .ok()?;
            key.get_value::<String, _>("Aria").ok()
        }

        fn set(&self, data: &str) {
            let (key, _) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
                .create_subkey(self.run())
                .unwrap();
            key.set_value("Aria", &data.to_owned()).unwrap();
        }
    }

    #[cfg(windows)]
    impl Drop for TestKey {
        fn drop(&mut self) {
            let _ =
                winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey_all(&self.0);
        }
    }

    #[cfg(windows)]
    fn registry(key: &TestKey, data: &str) -> Entry {
        Entry::Registry {
            subkey: key.run(),
            name: "Aria".to_owned(),
            data: data.to_owned(),
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_registry_entry_is_written_and_removed() {
        let key = TestKey::new();
        registry(&key, r#""C:\a\aria.exe" --autostart"#)
            .apply(true)
            .unwrap();
        assert_eq!(
            key.value().as_deref(),
            Some(r#""C:\a\aria.exe" --autostart"#)
        );
        registry(&key, r#""D:\b\aria.exe" --autostart"#)
            .apply(true)
            .unwrap();
        assert_eq!(
            key.value().as_deref(),
            Some(r#""D:\b\aria.exe" --autostart"#)
        );
        registry(&key, r#""D:\b\aria.exe" --autostart"#)
            .apply(false)
            .unwrap();
        assert_eq!(key.value(), None);
        // Removing again, or under a key that was never made, is fine.
        registry(&key, "x --autostart").apply(false).unwrap();
        let missing = TestKey::new();
        registry(&missing, "x --autostart").apply(false).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_foreign_value_is_left_alone() {
        let key = TestKey::new();
        key.set(r#""C:\Other\aria.exe""#);
        registry(&key, r#""C:\a\aria.exe" --autostart"#)
            .apply(false)
            .unwrap();
        assert_eq!(key.value().as_deref(), Some(r#""C:\Other\aria.exe""#));
    }
}
