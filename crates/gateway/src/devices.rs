//! Devices (phones and browsers) that have been paired with this host.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    /// Base64 X25519 public key.
    pub public_key: String,
    /// Unix seconds.
    pub paired_at: u64,
    pub last_seen: Option<u64>,
    /// Devices paired before roles existed were paired by the owner at the
    /// computer, so they are admins.
    #[serde(default = "DeviceRole::admin")]
    pub role: DeviceRole,
}

/// What a paired device may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRole {
    /// Everything: models, skills, settings, devices and remote access.
    Admin,
    /// Chat, and see which model and skills are in use.
    Member,
}

impl DeviceRole {
    fn admin() -> Self {
        DeviceRole::Admin
    }
}

pub struct DeviceStore {
    path: PathBuf,
    devices: RwLock<Vec<Device>>,
}

impl DeviceStore {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let devices = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        Ok(DeviceStore {
            path: path.to_path_buf(),
            devices: RwLock::new(devices),
        })
    }

    pub fn list(&self) -> Vec<Device> {
        self.devices.read().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Device> {
        self.devices
            .read()
            .unwrap()
            .iter()
            .find(|d| d.id == id)
            .cloned()
    }

    pub fn add(&self, device: Device) -> std::io::Result<()> {
        let mut devices = self.devices.write().unwrap();
        // Re-pairing the same phone replaces its old entry.
        devices.retain(|d| d.public_key != device.public_key);
        devices.push(device);
        self.save(&devices)
    }

    pub fn remove(&self, id: &str) -> std::io::Result<bool> {
        let mut devices = self.devices.write().unwrap();
        let before = devices.len();
        devices.retain(|d| d.id != id);
        let removed = devices.len() != before;
        if removed {
            self.save(&devices)?;
        }
        Ok(removed)
    }

    /// Changes a device's role. Returns false if there is no such device.
    pub fn set_role(&self, id: &str, role: DeviceRole) -> std::io::Result<bool> {
        let mut devices = self.devices.write().unwrap();
        let Some(d) = devices.iter_mut().find(|d| d.id == id) else {
            return Ok(false);
        };
        d.role = role;
        self.save(&devices)?;
        Ok(true)
    }

    /// Renames a device. Returns false if there is no such device.
    pub fn rename(&self, id: &str, name: &str) -> std::io::Result<bool> {
        let mut devices = self.devices.write().unwrap();
        let Some(d) = devices.iter_mut().find(|d| d.id == id) else {
            return Ok(false);
        };
        d.name = name.chars().take(64).collect();
        self.save(&devices)?;
        Ok(true)
    }

    /// Records activity in memory; persisted with the next change.
    pub fn touch(&self, id: &str, now: u64) {
        if let Some(d) = self
            .devices
            .write()
            .unwrap()
            .iter_mut()
            .find(|d| d.id == id)
        {
            d.last_seen = Some(now);
        }
    }

    fn save(&self, devices: &[Device]) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(devices)?)?;
        std::fs::rename(tmp, &self.path)
    }
}
