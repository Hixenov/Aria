//! The output the player uses: exclusive WASAPI when that is asked for and the device allows it,
//! the shared CPAL output otherwise.

use crate::cpal_output::CpalFactory;
use crate::device::DeviceId;
use crate::error::AudioError;
use crate::output::{DeviceLister, Opened, OutputFactory, OutputRequest};
use crate::sample_format::SourceFormat;

/// Opens outputs on this system's devices.
///
/// A shared request, or any request off Windows, goes to [`CpalFactory`]. An exclusive request on
/// Windows tries the WASAPI exclusive output first, and when the device refuses (another program
/// holds it, exclusive mode is not allowed, no format fits the track) opens it shared through
/// [`CpalFactory`], answering `exclusive: false`. Both open the same device for the same
/// [`DeviceId`].
pub struct SystemFactory;

/// Opens a device exclusively for a track, as [`crate::wasapi_output::open`] does.
type OpenExclusive<'a> =
    &'a dyn Fn(Option<&DeviceId>, Option<SourceFormat>) -> Result<Opened, AudioError>;

impl OutputFactory for SystemFactory {
    fn device_lister(&self) -> DeviceLister {
        CpalFactory.device_lister()
    }

    fn default_device(&self) -> Option<DeviceId> {
        CpalFactory.default_device()
    }

    fn open(
        &self,
        device: Option<&DeviceId>,
        request: OutputRequest,
    ) -> Result<Opened, AudioError> {
        #[cfg(windows)]
        let exclusive: Option<OpenExclusive<'_>> = Some(&crate::wasapi_output::open);
        #[cfg(not(windows))]
        let exclusive: Option<OpenExclusive<'_>> = None;
        route(device, request, exclusive, &CpalFactory)
    }
}

/// Send `request` to `exclusive` when it asks for exclusive mode and there is an exclusive
/// output, and to `shared`, with `exclusive: false`, otherwise or when the exclusive open fails.
fn route(
    device: Option<&DeviceId>,
    request: OutputRequest,
    exclusive: Option<OpenExclusive<'_>>,
    shared: &dyn OutputFactory,
) -> Result<Opened, AudioError> {
    if request.exclusive
        && let Some(open_exclusive) = exclusive
    {
        match open_exclusive(device, request.source) {
            Ok(opened) => return Ok(opened),
            Err(error) => {
                tracing::debug!(%error, "exclusive output not available; opening shared");
            }
        }
    }
    shared.open(
        device,
        OutputRequest {
            exclusive: false,
            ..request
        },
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::fake::FakeFactory;
    use crate::output::OutputFormat;
    use crate::sample_format::{DeviceFormat, SampleKind};

    const STEREO_48K: OutputFormat = OutputFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    const CD: SourceFormat = SourceFormat {
        sample_rate: 44_100,
        channels: 2,
        bits: Some(16),
        float: false,
    };
    const EXCLUSIVE_CD: OutputRequest = OutputRequest {
        exclusive: true,
        source: Some(CD),
    };

    /// An exclusive output that records what it was asked for and answers with an exclusive
    /// fake output, or, when `refuse` is set, a refusal.
    struct Exclusive {
        asked: RefCell<Vec<(Option<DeviceId>, Option<SourceFormat>)>>,
        factory: FakeFactory,
        refuse: bool,
    }

    impl Exclusive {
        fn new(refuse: bool) -> Self {
            Self {
                asked: RefCell::default(),
                factory: FakeFactory::new(STEREO_48K),
                refuse,
            }
        }

        fn open(
            &self,
            device: Option<&DeviceId>,
            source: Option<SourceFormat>,
        ) -> Result<Opened, AudioError> {
            self.asked.borrow_mut().push((device.cloned(), source));
            if self.refuse {
                return Err(AudioError::DeviceBusy);
            }
            self.factory.open(
                device,
                OutputRequest {
                    exclusive: true,
                    source,
                },
            )
        }
    }

    #[test]
    fn a_shared_request_goes_to_the_shared_output() {
        let exclusive = Exclusive::new(false);
        let shared = FakeFactory::new(STEREO_48K);
        let open = |device: Option<&DeviceId>, source| exclusive.open(device, source);

        let opened = route(None, OutputRequest::default(), Some(&open), &shared).unwrap();

        assert!(!opened.exclusive);
        assert!(exclusive.asked.borrow().is_empty());
        assert_eq!(shared.outputs().len(), 1);
    }

    #[test]
    fn an_exclusive_request_tries_the_exclusive_output_first() {
        let exclusive = Exclusive::new(false);
        let shared = FakeFactory::new(STEREO_48K);
        let open = |device: Option<&DeviceId>, source| exclusive.open(device, source);
        let device = DeviceId("wasapi:{endpoint}".into());

        let opened = route(Some(&device), EXCLUSIVE_CD, Some(&open), &shared).unwrap();

        assert!(opened.exclusive);
        assert_eq!(
            opened.device,
            DeviceFormat {
                sample_rate: 44_100,
                channels: 2,
                kind: SampleKind::I16
            }
        );
        assert_eq!(*exclusive.asked.borrow(), [(Some(device), Some(CD))]);
        assert!(shared.outputs().is_empty());
    }

    #[test]
    fn a_refused_exclusive_open_falls_back_to_shared() {
        let exclusive = Exclusive::new(true);
        let shared = FakeFactory::new(STEREO_48K);
        let open = |device: Option<&DeviceId>, source| exclusive.open(device, source);
        let device = DeviceId("wasapi:{endpoint}".into());

        let opened = route(Some(&device), EXCLUSIVE_CD, Some(&open), &shared).unwrap();

        assert!(!opened.exclusive);
        assert_eq!(exclusive.asked.borrow().len(), 1);
        // The shared output was asked for the same device, and not exclusively: the fake would
        // have opened it exclusively otherwise.
        let outputs = shared.outputs();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].device(), Some(device));
        assert!(!outputs[0].opened_as().0);
    }

    #[test]
    fn without_an_exclusive_output_every_request_is_shared() {
        let shared = FakeFactory::new(STEREO_48K);

        let opened = route(None, EXCLUSIVE_CD, None, &shared).unwrap();

        assert!(!opened.exclusive);
        assert!(!shared.outputs()[0].opened_as().0);
    }

    #[test]
    fn a_shared_failure_after_a_refusal_is_reported() {
        let exclusive = Exclusive::new(true);
        let shared = FakeFactory::new(STEREO_48K);
        shared.fail_open(1);
        let open = |device: Option<&DeviceId>, source| exclusive.open(device, source);

        assert!(route(None, EXCLUSIVE_CD, Some(&open), &shared).is_err());
    }
}
