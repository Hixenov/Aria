//! Lossless path <-> bytes codec for database storage.

use std::path::{Path, PathBuf};

/// Encodes a path as bytes: raw OS bytes on Unix, UTF-16 little-endian on Windows.
#[cfg(unix)]
pub(crate) fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

/// Encodes a path as bytes: raw OS bytes on Unix, UTF-16 little-endian on Windows.
#[cfg(windows)]
pub(crate) fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// Inverse of [`encode_path`].
#[cfg(unix)]
pub(crate) fn decode_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

/// Inverse of [`encode_path`].
#[cfg(windows)]
pub(crate) fn decode_path(bytes: &[u8]) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    PathBuf::from(std::ffi::OsString::from_wide(&units))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_round_trips_unicode() {
        for s in ["/музыка/Альбом", "/Երաժշտություն/ձայն", "/music/🎵/a.flac"]
        {
            let p = Path::new(s);
            assert_eq!(decode_path(&encode_path(p)), p);
        }
    }

    #[cfg(windows)]
    #[test]
    fn codec_keeps_unpaired_surrogate() {
        use std::os::windows::ffi::OsStringExt;
        let p = PathBuf::from(std::ffi::OsString::from_wide(&[0x43, 0xD800, 0x44]));
        assert_eq!(decode_path(&encode_path(&p)), p);
    }

    #[cfg(unix)]
    #[test]
    fn codec_keeps_non_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let p = PathBuf::from(std::ffi::OsStr::from_bytes(b"/m/\xff\xfe.flac"));
        assert_eq!(decode_path(&encode_path(&p)), p);
    }
}
