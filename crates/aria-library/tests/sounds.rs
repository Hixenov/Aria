//! Tests for the soundboard's stored sounds.

use std::path::{Path, PathBuf};

use aria_library::{Library, LibraryError, SOUND_NAME_MAX, SOUNDS_MAX, SoundId, sound_name};

/// Unwraps in helpers, which clippy's allow-expect-in-tests does not cover.
fn ok<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("test setup failed: {e:?}"),
    }
}

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: ok(tempfile::tempdir()),
        }
    }
    fn open(&self) -> Library {
        ok(Library::open(
            &self.dir.path().join("library.db"),
            &self.dir.path().join("art"),
        ))
        .library
    }
}

fn p(name: &str) -> PathBuf {
    Path::new("sounds").join(name)
}

fn names(lib: &Library) -> Vec<String> {
    ok(lib.sounds()).into_iter().map(|s| s.name).collect()
}

#[test]
fn sounds_keep_the_order_they_were_added_in() {
    let env = Env::new();
    let mut lib = env.open();
    assert_eq!(ok(lib.add_sounds(&[p("b.wav"), p("a.wav")])), 2);
    assert_eq!(ok(lib.add_sounds(&[p("c.wav")])), 1);
    assert_eq!(names(&lib), vec!["b", "a", "c"]);
    let sounds = ok(lib.sounds());
    assert_eq!(sounds[0].path, p("b.wav"));
    assert!(sounds[0].id.0 < sounds[1].id.0);
}

#[test]
fn the_default_name_is_the_file_stem() {
    let env = Env::new();
    let mut lib = env.open();
    ok(lib.add_sounds(&[p("airhorn.final.mp3"), p("Звонок.ogg"), p("noext")]));
    assert_eq!(names(&lib), vec!["airhorn.final", "Звонок", "noext"]);
    let s = &ok(lib.sounds())[0];
    assert_eq!(s.volume, 1.0);
    assert_eq!(s.hotkey, None);
}

#[test]
fn a_long_file_stem_is_cut_to_fifty_characters() {
    let env = Env::new();
    let mut lib = env.open();
    ok(lib.add_sounds(&[p(&format!("{}.wav", "я".repeat(80)))]));
    assert_eq!(names(&lib)[0].chars().count(), SOUND_NAME_MAX);
}

#[test]
fn names_are_trimmed_and_measured_in_characters() {
    assert_eq!(sound_name("  boom  "), Some("boom".to_owned()));
    assert_eq!(sound_name(""), None);
    assert_eq!(sound_name("   \t "), None);
    assert!(sound_name(&"a".repeat(50)).is_some());
    assert_eq!(sound_name(&"a".repeat(51)), None);
    // Armenian letters are two bytes, emoji four: characters are what count.
    assert!(sound_name(&"ձ".repeat(50)).is_some());
    assert_eq!(sound_name(&"ձ".repeat(51)), None);
    assert!(sound_name(&"🎵".repeat(50)).is_some());
    assert_eq!(sound_name(&"🎵".repeat(51)), None);

    let env = Env::new();
    let mut lib = env.open();
    ok(lib.add_sounds(&[p("x.wav")]));
    let id = ok(lib.sounds())[0].id;
    assert!(ok(lib.rename_sound(id, "  Air horn ")));
    assert_eq!(names(&lib), vec!["Air horn"]);
    for bad in ["", "  ", &"a".repeat(51)] {
        assert!(matches!(
            lib.rename_sound(id, bad),
            Err(LibraryError::InvalidSoundName)
        ));
    }
    assert_eq!(names(&lib), vec!["Air horn"]);
    assert!(ok(lib.rename_sound(id, &"🎵".repeat(50))));
}

#[test]
fn volume_is_clamped_to_zero_and_one() {
    let env = Env::new();
    let mut lib = env.open();
    ok(lib.add_sounds(&[p("x.wav")]));
    let id = ok(lib.sounds())[0].id;
    for (given, stored) in [
        (-1.0, 0.0),
        (2.0, 1.0),
        (f32::NAN, 1.0),
        (0.25, 0.25),
        (f32::INFINITY, 1.0),
        (f32::NEG_INFINITY, 0.0),
    ] {
        assert!(ok(lib.set_sound_volume(id, given)));
        assert_eq!(ok(lib.sounds())[0].volume, stored, "given {given}");
    }
}

#[test]
fn the_hotkey_is_stored_and_cleared() {
    let env = Env::new();
    let mut lib = env.open();
    ok(lib.add_sounds(&[p("x.wav")]));
    let id = ok(lib.sounds())[0].id;
    assert!(ok(lib.set_sound_hotkey(id, Some("Ctrl+Alt+1"))));
    assert_eq!(ok(lib.sounds())[0].hotkey.as_deref(), Some("Ctrl+Alt+1"));
    assert!(ok(lib.set_sound_hotkey(id, None)));
    assert_eq!(ok(lib.sounds())[0].hotkey, None);
}

#[test]
fn the_same_file_can_be_added_twice() {
    let env = Env::new();
    let mut lib = env.open();
    assert_eq!(ok(lib.add_sounds(&[p("x.wav"), p("x.wav")])), 2);
    assert_eq!(ok(lib.add_sounds(&[p("x.wav")])), 1);
    let sounds = ok(lib.sounds());
    assert_eq!(sounds.len(), 3);
    assert!(sounds.iter().all(|s| s.path == p("x.wav")));
}

#[test]
fn adding_stops_at_one_hundred() {
    let env = Env::new();
    let mut lib = env.open();
    let first: Vec<PathBuf> = (0..98).map(|i| p(&format!("{i}.wav"))).collect();
    assert_eq!(ok(lib.add_sounds(&first)), 98);
    let more: Vec<PathBuf> = (98..103).map(|i| p(&format!("{i}.wav"))).collect();
    assert_eq!(ok(lib.add_sounds(&more)), 2);
    let sounds = ok(lib.sounds());
    assert_eq!(sounds.len(), SOUNDS_MAX);
    assert_eq!(sounds[99].name, "99");
    assert_eq!(ok(lib.add_sounds(&[p("z.wav")])), 0);
    assert_eq!(ok(lib.sounds()).len(), SOUNDS_MAX);
}

#[test]
fn an_unknown_id_changes_nothing() {
    let env = Env::new();
    let mut lib = env.open();
    ok(lib.add_sounds(&[p("x.wav")]));
    let before = ok(lib.sounds());
    let unknown = SoundId(9_999);
    assert!(!ok(lib.rename_sound(unknown, "y")));
    assert!(!ok(lib.set_sound_volume(unknown, 0.5)));
    assert!(!ok(lib.set_sound_hotkey(unknown, Some("F5"))));
    assert!(!ok(lib.remove_sound(unknown)));
    assert_eq!(ok(lib.sounds()), before);
}

#[test]
fn removing_a_sound_keeps_the_others_in_order() {
    let env = Env::new();
    let mut lib = env.open();
    ok(lib.add_sounds(&[p("a.wav"), p("b.wav"), p("c.wav")]));
    let id = ok(lib.sounds())[1].id;
    assert!(ok(lib.remove_sound(id)));
    assert!(!ok(lib.remove_sound(id)));
    assert_eq!(names(&lib), vec!["a", "c"]);
    ok(lib.add_sounds(&[p("d.wav")]));
    assert_eq!(names(&lib), vec!["a", "c", "d"]);
}

#[test]
fn sounds_survive_closing_and_opening() {
    let env = Env::new();
    let mut lib = env.open();
    ok(lib.add_sounds(&[p("a.wav"), p("b.wav")]));
    let id = ok(lib.sounds())[0].id;
    ok(lib.rename_sound(id, "Первый"));
    ok(lib.set_sound_volume(id, 0.5));
    ok(lib.set_sound_hotkey(id, Some("Ctrl+F1")));
    let before = ok(lib.sounds());
    drop(lib);
    let lib = env.open();
    assert_eq!(ok(lib.sounds()), before);
    assert_eq!(before[0].volume, 0.5);
}
