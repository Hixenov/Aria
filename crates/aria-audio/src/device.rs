//! Finding audio devices.

use std::fmt::Display;
use std::str::FromStr;

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

use crate::error::AudioError;

/// Marks an identifier made from the device name, for devices that have no ID of their own.
const NAME_PREFIX: &str = "name:";

/// Starts the identifier of a device on CPAL's WASAPI host. `cpal::DeviceId` displays as
/// `"<host>:<id>"`, and on this host the rest is the WASAPI endpoint ID itself
/// (`IMMDevice::GetId`).
#[cfg(windows)]
const WASAPI_PREFIX: &str = "wasapi:";

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

/// The identifier of the system default output device, found without listing the others. It is
/// made as [`list_devices`] makes it, so the two can be compared.
pub(crate) fn default_output_id() -> Option<DeviceId> {
    let device = cpal::default_host().default_output_device()?;
    describe(&device).map(|(id, _)| id)
}

/// Find an output device, or the system default when `id` is `None`.
pub(crate) fn find_output_device(
    host: &cpal::Host,
    id: Option<&DeviceId>,
) -> Result<cpal::Device, AudioError> {
    find_device(host, DeviceKind::Output, id)
}

/// Find a device of `kind`, or the system default for that kind when `id` is `None`.
pub(crate) fn find_device(
    host: &cpal::Host,
    kind: DeviceKind,
    id: Option<&DeviceId>,
) -> Result<cpal::Device, AudioError> {
    let Some(id) = id else {
        let default = match kind {
            DeviceKind::Input => host.default_input_device(),
            DeviceKind::Output => host.default_output_device(),
        };
        return default.ok_or(AudioError::NoDevice);
    };
    let found = match id.0.strip_prefix(NAME_PREFIX) {
        Some(name) => {
            let devices = match kind {
                DeviceKind::Input => host.input_devices(),
                DeviceKind::Output => host.output_devices(),
            };
            devices
                .map_err(device_error)?
                .find(|device| describe(device).is_some_and(|(_, found)| found == name))
        }
        None => cpal::DeviceId::from_str(&id.0)
            .ok()
            .and_then(|id| host.device_by_id(&id)),
    };
    found.ok_or_else(|| AudioError::DeviceNotFound(id.0.clone()))
}

/// An output endpoint as WASAPI lists it.
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    /// The endpoint ID (`IMMDevice::GetId`).
    pub(crate) id: String,
    /// The friendly name, the same text CPAL shows as the device name.
    pub(crate) name: String,
}

/// The endpoint a saved identifier names, so the exclusive output opens the same device as the
/// shared one. A CPAL identifier carries the endpoint ID, which is matched exactly; one CPAL made
/// from the name, for a device that had no ID, is matched by the friendly name.
#[cfg(windows)]
pub(crate) fn find_endpoint<'a>(
    wanted: &DeviceId,
    endpoints: &'a [Endpoint],
) -> Option<&'a Endpoint> {
    if let Some(id) = wanted.0.strip_prefix(WASAPI_PREFIX) {
        return endpoints.iter().find(|endpoint| endpoint.id == id);
    }
    let name = wanted.0.strip_prefix(NAME_PREFIX)?;
    endpoints.iter().find(|endpoint| endpoint.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_without_an_id_falls_back_to_its_name() {
        let id = stable_id(Err(cpal::ErrorKind::DeviceNotAvailable.into()), "Speakers");

        assert_eq!(id, DeviceId("name:Speakers".into()));
    }

    /// The two output endpoints of the PC this was written on, as WASAPI listed them. CPAL
    /// listed the same devices as `wasapi:` followed by these IDs, with the same names.
    #[cfg(windows)]
    fn endpoints() -> Vec<Endpoint> {
        vec![
            Endpoint {
                id: "{0.0.0.00000000}.{a19a14b8-5189-4ca8-b854-cd2fe0eb0e7a}".into(),
                name: "Наушники (Realtek(R) Audio)".into(),
            },
            Endpoint {
                id: "{0.0.0.00000000}.{aabc0222-cba4-4b92-8686-0f7284536b67}".into(),
                name: "Динамики (Realtek(R) Audio)".into(),
            },
        ]
    }

    #[cfg(windows)]
    #[test]
    fn a_stored_device_id_finds_its_endpoint() {
        let endpoints = endpoints();
        let stored =
            DeviceId("wasapi:{0.0.0.00000000}.{aabc0222-cba4-4b92-8686-0f7284536b67}".into());

        assert_eq!(find_endpoint(&stored, &endpoints), Some(&endpoints[1]));

        // The ID decides, even when another endpoint has the same name.
        let mut same_names = endpoints.clone();
        same_names[0].name = same_names[1].name.clone();
        assert_eq!(find_endpoint(&stored, &same_names), Some(&same_names[1]));

        // An endpoint that is gone is not found, so the caller falls back to the default.
        let gone =
            DeviceId("wasapi:{0.0.0.00000000}.{00000000-0000-0000-0000-000000000000}".into());
        assert_eq!(find_endpoint(&gone, &endpoints), None);
    }

    #[cfg(windows)]
    #[test]
    fn an_unknown_id_falls_back_to_the_name() {
        let endpoints = endpoints();

        let by_name = DeviceId("name:Наушники (Realtek(R) Audio)".into());
        assert_eq!(find_endpoint(&by_name, &endpoints), Some(&endpoints[0]));

        let no_such_name = DeviceId("name:Speakers".into());
        assert_eq!(find_endpoint(&no_such_name, &endpoints), None);

        // An identifier from another host names no endpoint.
        let other_host = DeviceId("asio:Realtek ASIO".into());
        assert_eq!(find_endpoint(&other_host, &endpoints), None);
    }
}
