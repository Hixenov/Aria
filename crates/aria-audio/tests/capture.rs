//! The capture source: the fake that engine tests use, and a real microphone for a person only.

use aria_audio::fake::FakeCapture;
use aria_audio::{AudioError, CaptureFactory, CaptureFormat, CpalCapture, DeviceId, DeviceInfo};

const MONO_48K: CaptureFormat = CaptureFormat {
    sample_rate: 48_000,
    channels: 1,
};

#[test]
fn the_fake_capture_delivers_what_is_pushed() {
    let (capture, control) = FakeCapture::new(MONO_48K);
    assert!(!control.open_now());

    let mut reader = capture.open(None).expect("the fake opens");
    assert!(control.open_now());
    assert_eq!(control.opened(), 1);
    assert_eq!(reader.format(), MONO_48K);

    assert_eq!(control.push(&[0.25, -0.5, 0.75]), 3);
    let mut out = [0.0; 8];
    assert_eq!(reader.read(&mut out), 3);
    assert_eq!(&out[..3], &[0.25, -0.5, 0.75]);
    assert_eq!(reader.read(&mut out), 0);
    assert!(!reader.failed());

    control.fail();
    assert!(reader.failed());

    drop(reader);
    assert!(!control.open_now());
}

#[test]
fn a_refused_open_is_an_error() {
    let (capture, control) = FakeCapture::new(MONO_48K);

    control.fail_open(true);
    assert!(matches!(capture.open(None), Err(AudioError::Device(_))));
    control.deny(true);
    assert!(matches!(
        capture.open(None),
        Err(AudioError::PermissionDenied)
    ));
    assert!(!control.open_now());
    assert_eq!(control.opened(), 0);

    control.fail_open(false);
    control.deny(false);
    let reader = capture.open(None).expect("opens once allowed");
    assert!(control.open_now());
    drop(reader);
}

#[test]
fn pushing_with_nothing_open_goes_nowhere() {
    let (_capture, control) = FakeCapture::new(MONO_48K);

    assert_eq!(control.push(&[0.5; 4]), 0);
}

#[test]
#[ignore = "opens a real microphone; run by hand only"]
fn the_default_microphone_opens() {
    let mut reader = CpalCapture
        .open(None)
        .expect("the default microphone opens");
    let format = reader.format();
    assert!(
        (8_000..=384_000).contains(&format.sample_rate),
        "{format:?}"
    );
    assert!((1..=8).contains(&format.channels), "{format:?}");

    // Read for 200 ms and throw it all away: nothing captured is printed or kept.
    let mut scratch = vec![0.0; 4096];
    let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while std::time::Instant::now() < until {
        reader.read(&mut scratch);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    scratch.fill(0.0);
    assert!(!reader.failed());
}

#[test]
fn a_cable_is_never_opened_as_the_microphone() {
    let (capture, control) = FakeCapture::new(MONO_48K);
    let cable = DeviceInfo {
        id: DeviceId("cable".into()),
        name: "CABLE Output (VB-Audio Virtual Cable)".into(),
        is_default: true,
    };
    let mic = DeviceInfo {
        id: DeviceId("mic".into()),
        name: "Microphone (USB)".into(),
        is_default: false,
    };
    control.set_devices(vec![cable.clone(), mic.clone()]);

    // Chosen, or as the system default.
    assert!(matches!(
        capture.open(Some(&cable.id)),
        Err(AudioError::CableAsMicrophone)
    ));
    assert!(matches!(
        capture.open(None),
        Err(AudioError::CableAsMicrophone)
    ));
    assert!(!control.open_now());

    let reader = capture
        .open(Some(&mic.id))
        .expect("a real microphone opens");
    assert!(control.open_now());
    drop(reader);
}
