//! The output the player uses: exclusive WASAPI when that is asked for and the device allows it,
//! the shared CPAL output otherwise.

use crate::cpal_output::CpalFactory;
use crate::device::DeviceId;
use crate::error::AudioError;
use crate::output::{DeviceLister, Opened, OutputFactory, OutputRequest};

/// Opens outputs on this system's devices.
///
/// A shared request, or any request off Windows, goes to [`CpalFactory`]. An exclusive request on
/// Windows tries the WASAPI exclusive output first, and when the device refuses (another program
/// holds it, exclusive mode is not allowed, no format fits the track) opens it shared through
/// [`CpalFactory`], answering `exclusive: false`. Both open the same device for the same
/// [`DeviceId`].
pub struct SystemFactory;

impl OutputFactory for SystemFactory {
    fn device_lister(&self) -> DeviceLister {
        CpalFactory.device_lister()
    }

    fn open(
        &self,
        device: Option<&DeviceId>,
        request: OutputRequest,
    ) -> Result<Opened, AudioError> {
        #[cfg(windows)]
        if request.exclusive {
            match crate::wasapi_output::open(device, request.source) {
                Ok(opened) => return Ok(opened),
                Err(error) => {
                    tracing::debug!(%error, "exclusive output not available; opening shared");
                }
            }
        }
        CpalFactory.open(
            device,
            OutputRequest {
                exclusive: false,
                ..request
            },
        )
    }
}
