//! Outputs and devices, driven through the fake output.

use aria_audio::fake::FakeFactory;
use aria_audio::{DeviceId, DeviceKind, OutputFactory, OutputFormat, list_devices};

const STEREO_48K: OutputFormat = OutputFormat {
    sample_rate: 48_000,
    channels: 2,
};
const MONO_44K: OutputFormat = OutputFormat {
    sample_rate: 44_100,
    channels: 1,
};

#[test]
fn several_outputs_run_at_once() {
    let music = FakeFactory::new(STEREO_48K);
    let cable = FakeFactory::new(MONO_44K);
    let mut music_output = music.open(None).unwrap();
    let mut cable_output = cable.open(None).unwrap();
    assert_eq!(music_output.format(), STEREO_48K);
    assert_eq!(cable_output.format(), MONO_44K);

    let mut music_input = music_output.add_input(1_024).unwrap();
    let mut cable_input = cable_output.add_input(1_024).unwrap();
    music_input.write(&[0.25; 256]);
    cable_input.write(&[0.75; 256]);

    let heard_music = music.outputs()[0].pull(64);
    assert_eq!(heard_music, vec![0.25; 128]);
    assert_eq!(music_input.consumed(), 128);
    assert_eq!(cable_input.consumed(), 0);

    let heard_cable = cable.outputs()[0].pull(64);
    assert_eq!(heard_cable, vec![0.75; 64]);
    assert_eq!(music_input.consumed(), 128);
    assert_eq!(cable_input.consumed(), 64);
}

#[test]
fn one_factory_opens_independent_outputs() {
    let factory = FakeFactory::new(MONO_44K);
    let mut first = factory.open(None).unwrap();
    let _second = factory.open(None).unwrap();
    let mut input = first.add_input(64).unwrap();
    input.write(&[0.5; 8]);

    let outputs = factory.outputs();
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[1].pull(8), vec![0.0; 8]);
    assert_eq!(outputs[0].pull(8), vec![0.5; 8]);
}

#[test]
fn open_failure_reports_error() {
    let factory = FakeFactory::new(STEREO_48K);
    factory.fail_open(1);

    assert!(factory.open(None).is_err());
    assert!(factory.outputs().is_empty());

    assert!(factory.open(None).is_ok());
    assert_eq!(factory.outputs().len(), 1);
}

#[test]
fn factory_clones_share_their_outputs() {
    let factory = FakeFactory::new(STEREO_48K);
    let boxed: Box<dyn OutputFactory> = Box::new(factory.clone());

    let _output = boxed.open(None).unwrap();

    assert_eq!(factory.outputs().len(), 1);
}

#[test]
fn control_reports_the_device_and_fails_the_output() {
    let factory = FakeFactory::new(STEREO_48K);
    let device = DeviceId("wasapi:speakers".into());
    let chosen = factory.open(Some(&device)).unwrap();
    let default = factory.open(None).unwrap();

    let outputs = factory.outputs();
    assert_eq!(outputs[0].device(), Some(device));
    assert_eq!(outputs[1].device(), None);
    assert!(!chosen.failed());

    outputs[0].fail();

    assert!(chosen.failed());
    assert!(!default.failed());
}

#[test]
fn list_devices_does_not_panic() {
    for kind in [DeviceKind::Input, DeviceKind::Output] {
        if let Ok(devices) = list_devices(kind) {
            assert!(devices.iter().filter(|device| device.is_default).count() <= 1);
            assert!(devices.iter().all(|device| !device.id.0.is_empty()));
        }
    }
}
