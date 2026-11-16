//! Outputs and devices, driven through the fake output.

use aria_audio::fake::FakeFactory;
use aria_audio::{
    AudioError, AudioOutput, DeviceFormat, DeviceId, DeviceKind, OutputFactory, OutputFormat,
    OutputRequest, SampleKind, SourceFormat, list_devices,
};

const STEREO_48K: OutputFormat = OutputFormat {
    sample_rate: 48_000,
    channels: 2,
};
const MONO_44K: OutputFormat = OutputFormat {
    sample_rate: 44_100,
    channels: 1,
};

/// A shared output, as every open was before exclusive mode.
fn open(
    factory: &FakeFactory,
    device: Option<&DeviceId>,
) -> Result<Box<dyn AudioOutput>, AudioError> {
    factory
        .open(device, OutputRequest::default())
        .map(|opened| opened.output)
}

fn exclusive_for(source: SourceFormat) -> OutputRequest {
    OutputRequest {
        exclusive: true,
        source: Some(source),
    }
}

const CD: SourceFormat = SourceFormat {
    sample_rate: 44_100,
    channels: 2,
    bits: Some(16),
    float: false,
};

#[test]
fn a_shared_open_runs_at_the_factory_format() {
    let factory = FakeFactory::new(STEREO_48K);

    let opened = factory.open(None, OutputRequest::default()).unwrap();

    let shared = DeviceFormat {
        sample_rate: 48_000,
        channels: 2,
        kind: SampleKind::I32,
    };
    assert!(!opened.exclusive);
    assert_eq!(opened.device, shared);
    assert_eq!(opened.output.format(), STEREO_48K);
    assert_eq!(factory.outputs()[0].opened_as(), (false, shared));
}

#[test]
fn a_fake_exclusive_open_takes_the_track_format() {
    let factory = FakeFactory::new(STEREO_48K);

    let opened = factory.open(None, exclusive_for(CD)).unwrap();

    let cd = DeviceFormat {
        sample_rate: 44_100,
        channels: 2,
        kind: SampleKind::I16,
    };
    assert!(opened.exclusive);
    assert_eq!(opened.device, cd);
    assert_eq!(
        opened.output.format(),
        OutputFormat {
            sample_rate: 44_100,
            channels: 2
        }
    );
    assert_eq!(factory.outputs()[0].opened_as(), (true, cd));
}

#[test]
fn a_fake_exclusive_open_keeps_to_what_the_device_takes() {
    let factory = FakeFactory::new(STEREO_48K);
    factory.set_exclusive_rates(vec![48_000, 96_000]);
    factory.set_exclusive_kinds(vec![SampleKind::I24In32]);

    let opened = factory.open(None, exclusive_for(CD)).unwrap();
    assert!(opened.exclusive);
    assert_eq!(
        opened.device,
        DeviceFormat {
            sample_rate: 48_000,
            channels: 2,
            kind: SampleKind::I24In32
        }
    );

    // Nothing at any rate: the open is shared, and does not fail.
    factory.set_exclusive_kinds(Vec::new());
    let opened = factory.open(None, exclusive_for(CD)).unwrap();
    assert!(!opened.exclusive);
    assert_eq!(opened.output.format(), STEREO_48K);
}

#[test]
fn a_fake_device_that_refuses_exclusive_opens_shared() {
    let factory = FakeFactory::new(STEREO_48K);
    factory.refuse_exclusive(true);

    let opened = factory.open(None, exclusive_for(CD)).unwrap();
    assert!(!opened.exclusive);
    assert_eq!(opened.output.format(), STEREO_48K);

    factory.refuse_exclusive(false);
    // No track: the device's own rate.
    let request = OutputRequest {
        exclusive: true,
        source: None,
    };
    let opened = factory.open(None, request).unwrap();
    assert!(opened.exclusive);
    assert_eq!(opened.device.sample_rate, 48_000);
}

#[test]
fn several_outputs_run_at_once() {
    let music = FakeFactory::new(STEREO_48K);
    let cable = FakeFactory::new(MONO_44K);
    let mut music_output = open(&music, None).unwrap();
    let mut cable_output = open(&cable, None).unwrap();
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
    let mut first = open(&factory, None).unwrap();
    let _second = open(&factory, None).unwrap();
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

    assert!(open(&factory, None).is_err());
    assert!(factory.outputs().is_empty());

    assert!(open(&factory, None).is_ok());
    assert_eq!(factory.outputs().len(), 1);
}

#[test]
fn factory_clones_share_their_outputs() {
    let factory = FakeFactory::new(STEREO_48K);
    let boxed: Box<dyn OutputFactory> = Box::new(factory.clone());

    let _output = boxed.open(None, OutputRequest::default()).unwrap();

    assert_eq!(factory.outputs().len(), 1);
}

#[test]
fn control_reports_the_device_and_fails_the_output() {
    let factory = FakeFactory::new(STEREO_48K);
    let device = DeviceId("wasapi:speakers".into());
    let chosen = open(&factory, Some(&device)).unwrap();
    let default = open(&factory, None).unwrap();

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
