//! `paired-devices.json` as Wristper writes it: `{"devices": [...]}`. Each
//! entry is read on its own (`compactMap` in `Pairing.swift`), and an entry
//! Hex cannot read is kept as it was and written back unchanged, so one bad
//! entry never costs the other watches or itself.

use std::io;
use std::path::Path;

use serde_json::{Map, Value};

use super::PairedDevice;

#[derive(Default)]
pub(super) struct DevicesFile {
    pub(super) devices: Vec<PairedDevice>,
    pub(super) unparsed: Vec<Value>,
}

/// A missing file is an empty list. `Err` is a file that exists but is not
/// a device list: nothing may overwrite it.
pub(super) fn read(path: &Path) -> Result<DevicesFile, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(DevicesFile::default()),
        Err(error) => return Err(format!("unreadable: {error}")),
    };
    let mut object: Map<String, Value> =
        serde_json::from_slice(&bytes).map_err(|error| format!("not a JSON object: {error}"))?;
    let Some(Value::Array(entries)) = object.remove("devices") else {
        return Err("no `devices` list".to_owned());
    };
    let mut file = DevicesFile::default();
    for entry in entries {
        match serde_json::from_value::<PairedDevice>(entry.clone()) {
            Ok(device) => file.devices.push(device),
            Err(_) => file.unparsed.push(entry),
        }
    }
    Ok(file)
}

/// The listed devices, then the entries `read` could not parse, except one
/// whose `id` a listed device now has (pairing again replaced it).
pub(super) fn write(path: &Path, devices: Vec<PairedDevice>, unparsed: &[Value]) -> io::Result<()> {
    let kept: Vec<Value> = unparsed
        .iter()
        .filter(|entry| {
            let id = entry.get("id").and_then(Value::as_str);
            !devices.iter().any(|device| Some(device.id.as_str()) == id)
        })
        .cloned()
        .collect();
    let mut entries = devices
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(io::Error::other)?;
    entries.extend(kept);
    let bytes =
        serde_json::to_vec(&serde_json::json!({ "devices": entries })).map_err(io::Error::other)?;
    super::super::write_owner_only(path, &bytes)
}
