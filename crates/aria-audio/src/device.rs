//! Finding audio devices.

use std::fmt::Display;
use std::str::FromStr;

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

use crate::error::AudioError;

/// Marks an identifier made from the device name, for devices that have no ID of their own.
const NAME_PREFIX: &str = "name:";

/// A device identifier that can be saved and looked up again later.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub String);

/// Which direction of device to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    /// Devices that capture audio.
    Input,
    /// Devices that play audio.
    Output,
}

/// One audio device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The identifier to save and open it by.
    pub id: DeviceId,
    /// The name to show.
    pub name: String,
    /// Whether this is the system default for its kind.
    pub is_default: bool,
}

pub(crate) fn device_error(error: impl Display) -> AudioError {
    AudioError::Device(error.to_string())
}

fn stable_id(id: Result<cpal::DeviceId, cpal::Error>, name: &str) -> DeviceId {
    match id {
        Ok(id) => DeviceId(id.to_string()),
        Err(_) => DeviceId(format!("{NAME_PREFIX}{name}")),
    }
}

/// The identifier and name of `device`, or `None` if it has just disconnected.
fn describe(device: &cpal::Device) -> Option<(DeviceId, String)> {
    let description = device.description().ok()?;
    let name = description.name().to_owned();
    Some((stable_id(device.id(), &name), name))
}

/// List the devices of one kind on the default host.
pub fn list_devices(kind: DeviceKind) -> Result<Vec<DeviceInfo>, AudioError> {
    let host = cpal::default_host();
    let (devices, default) = match kind {
        DeviceKind::Input => (host.input_devices(), host.default_input_device()),
        DeviceKind::Output => (host.output_devices(), host.default_output_device()),
    };
    let default_id = default.as_ref().and_then(describe).map(|(id, _)| id);
    let devices = devices.map_err(device_error)?;
    Ok(devices
        .filter_map(|device| describe(&device))
        .map(|(id, name)| DeviceInfo {
            is_default: default_id.as_ref() == Some(&id),
            id,
            name,
        })
        .collect())
}

/// Find an output device, or the system default when `id` is `None`.
pub(crate) fn find_output_device(
    host: &cpal::Host,
    id: Option<&DeviceId>,
) -> Result<cpal::Device, AudioError> {
    let Some(id) = id else {
        return host.default_output_device().ok_or(AudioError::NoDevice);
    };
    let found = match id.0.strip_prefix(NAME_PREFIX) {
        Some(name) => host
            .output_devices()
            .map_err(device_error)?
            .find(|device| describe(device).is_some_and(|(_, found)| found == name)),
        None => cpal::DeviceId::from_str(&id.0)
            .ok()
            .and_then(|id| host.device_by_id(&id)),
    };
    found.ok_or_else(|| AudioError::DeviceNotFound(id.0.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_without_an_id_falls_back_to_its_name() {
        let id = stable_id(Err(cpal::ErrorKind::DeviceNotAvailable.into()), "Speakers");

        assert_eq!(id, DeviceId("name:Speakers".into()));
    }
}
