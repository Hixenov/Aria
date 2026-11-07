//! Tag reading and the artwork cache, against the shared audio fixtures.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use aria_library::{
    ArtworkCache, LibraryError, MAX_IMAGE_BYTES, THUMB_SIZE, find_folder_image, read_tags,
};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, Tag, TagExt, TagType};

/// Unwraps in helpers, which clippy's allow-unwrap-in-tests does not cover.
fn ok<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("test setup failed: {e:?}"),
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../aria-audio/tests/fixtures")
        .join(name)
}

fn png_bytes(width: u32, height: u32, rgba: bool) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    if rgba {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 30, 30, 0]));
        ok(img.write_to(&mut out, image::ImageFormat::Png));
    } else {
        let img = image::RgbImage::from_pixel(width, height, image::Rgb([20, 90, 200]));
        ok(img.write_to(&mut out, image::ImageFormat::Png));
    }
    out.into_inner()
}

fn jpg_files(dir: &Path) -> Vec<PathBuf> {
    ok(fs::read_dir(dir))
        .map(|e| ok(e).path())
        .filter(|p| p.extension().is_some_and(|e| e == "jpg"))
        .collect()
}

#[test]
fn reads_tags() {
    let t = read_tags(&fixture("tagged.mp3")).unwrap();
    assert_eq!(t.title, "Fixture Title");
    assert_eq!(t.artist.as_deref(), Some("Fixture Artist"));
    assert_eq!(t.album.as_deref(), Some("Fixture Album"));
    assert_eq!(t.track_no, Some(3));
    assert!(
        (900..=1100).contains(&t.duration_ms),
        "duration {}",
        t.duration_ms
    );
    assert!(t.sample_rate > 0);
    assert!(t.channels > 0);
    assert!(!t.codec.is_empty());
}

#[test]
fn missing_tags_fall_back_to_file_name() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("Моя песня.wav");
    fs::copy(fixture("tone.wav"), &copy).unwrap();
    let t = read_tags(&copy).unwrap();
    assert_eq!(t.title, "Моя песня");
    assert_eq!(t.artist, None);
    assert!(t.duration_ms > 0);
}

#[test]
fn blank_tags_are_none() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("blank.mp3");
    fs::copy(fixture("tone.mp3"), &copy).unwrap();
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title("   ".to_owned());
    tag.set_artist(String::new());
    tag.set_album("\t \n".to_owned());
    tag.set_genre(" ".to_owned());
    tag.save_to_path(&copy, WriteOptions::default()).unwrap();
    // Make sure a tag really is in the file, so the blanks are what is being tested.
    let probed = lofty::read_from_path(&copy).unwrap();
    assert!(probed.primary_tag().is_some());

    let t = read_tags(&copy).unwrap();
    assert_eq!(t.title, "blank");
    assert_eq!(t.artist, None);
    assert_eq!(t.album, None);
    assert_eq!(t.genre, None);
}

#[test]
fn garbage_file_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.mp3");
    fs::write(&bad, [0xFFu8, 0xFB, 0x00, 0x13, 0x37, 1, 2, 3]).unwrap();
    let _ = read_tags(&bad); // either result is fine; it must not panic
    let empty = dir.path().join("empty.flac");
    fs::write(&empty, b"").unwrap();
    assert!(matches!(read_tags(&empty), Err(LibraryError::Tags(_))));
    assert!(read_tags(&dir.path().join("missing.mp3")).is_err());
}

#[test]
fn embedded_picture_is_read() {
    let t = read_tags(&fixture("cover.flac")).unwrap();
    let pic = t.picture.expect("cover.flac has a picture");
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtworkCache::new(dir.path().join("art"));
    let id = cache.store(&pic).unwrap().expect("stored");
    let thumb = image::open(cache.path_of(&id)).unwrap();
    assert_eq!(thumb.width().max(thumb.height()), THUMB_SIZE);
    assert!(read_tags(&fixture("tone.flac")).unwrap().picture.is_none());
}

#[test]
fn same_image_is_stored_once() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtworkCache::new(dir.path().to_path_buf());
    let bytes = png_bytes(400, 300, false);
    let a = cache.store(&bytes).unwrap().unwrap();
    let b = cache.store(&bytes).unwrap().unwrap();
    assert_eq!(a, b);
    assert_eq!(a.0.len(), 32);
    assert_eq!(jpg_files(dir.path()).len(), 1);
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        1,
        "no temp files"
    );
}

#[test]
fn small_images_are_not_upscaled() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtworkCache::new(dir.path().to_path_buf());
    let id = cache.store(&png_bytes(64, 32, false)).unwrap().unwrap();
    let thumb = image::open(cache.path_of(&id)).unwrap();
    assert_eq!((thumb.width(), thumb.height()), (64, 32));
}

#[test]
fn transparent_png_is_stored() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtworkCache::new(dir.path().to_path_buf());
    let id = cache.store(&png_bytes(300, 300, true)).unwrap().unwrap();
    assert!(cache.path_of(&id).is_file());
}

#[test]
fn missing_cache_dir_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a").join("b");
    let cache = ArtworkCache::new(nested.clone());
    cache.store(&png_bytes(10, 10, false)).unwrap().unwrap();
    assert_eq!(jpg_files(&nested).len(), 1);
}

#[test]
fn oversized_image_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtworkCache::new(dir.path().to_path_buf());
    assert!(cache.store(&png_bytes(9000, 1, false)).unwrap().is_none());
    assert!(cache.store(&png_bytes(1, 9000, false)).unwrap().is_none());
    let too_many_bytes = vec![0u8; MAX_IMAGE_BYTES + 1];
    assert!(cache.store(&too_many_bytes).unwrap().is_none());
    assert!(jpg_files(dir.path()).is_empty());
}

#[test]
fn garbage_image_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtworkCache::new(dir.path().to_path_buf());
    assert!(cache.store(b"definitely not an image").unwrap().is_none());
    assert!(cache.store(b"").unwrap().is_none());
    // A truncated but valid-looking PNG.
    let png = png_bytes(300, 300, false);
    assert!(cache.store(&png[..png.len() / 2]).unwrap().is_none());
    assert!(jpg_files(dir.path()).is_empty());
}

#[test]
fn folder_image_order() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(find_folder_image(dir.path()), None);
    fs::write(dir.path().join("other.jpg"), b"x").unwrap();
    assert_eq!(find_folder_image(dir.path()), None);
    fs::write(dir.path().join("front.png"), b"x").unwrap();
    assert_eq!(
        find_folder_image(dir.path()).unwrap().file_name().unwrap(),
        "front.png"
    );
    fs::write(dir.path().join("Folder.jpeg"), b"x").unwrap();
    assert_eq!(
        find_folder_image(dir.path()).unwrap().file_name().unwrap(),
        "Folder.jpeg"
    );
    fs::write(dir.path().join("Cover.JPG"), b"x").unwrap();
    assert_eq!(
        find_folder_image(dir.path()).unwrap().file_name().unwrap(),
        "Cover.JPG"
    );
    assert_eq!(find_folder_image(&dir.path().join("nope")), None);
}

#[test]
fn clear_removes_thumbnails() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtworkCache::new(dir.path().to_path_buf());
    let id = cache.store(&png_bytes(50, 50, false)).unwrap().unwrap();
    fs::write(dir.path().join("keep.txt"), b"x").unwrap();
    cache.clear().unwrap();
    assert!(!cache.path_of(&id).exists());
    assert!(dir.path().join("keep.txt").exists());
    // A missing directory is fine.
    ArtworkCache::new(dir.path().join("gone")).clear().unwrap();
    // Storing again after clearing rebuilds the file.
    let again = cache.store(&png_bytes(50, 50, false)).unwrap().unwrap();
    assert_eq!(again, id);
    assert!(cache.path_of(&again).is_file());
}
