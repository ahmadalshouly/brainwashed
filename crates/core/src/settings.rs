use brainwashed_runtime::{Backend, SamplingOptions};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You are BrainWashed, a helpful assistant running privately on the user's own computer. \
Answer clearly and concisely.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Name shown to paired phones. Defaults to the computer's hostname.
    pub host_name: Option<String>,
    pub backend: Backend,
    pub context_size: u32,
    pub gpu_layers: u32,
    /// Use this llama-server instead of downloading one.
    pub llama_server_path: Option<PathBuf>,
    /// Model loaded at startup.
    pub active_model: Option<String>,
    pub system_prompt: String,
    /// Skills the user switched off.
    pub disabled_skills: Vec<String>,
    /// Unused since the desktop app was removed: the `brainwashed` command
    /// always serves. Kept so older settings files still load.
    pub phone_access: bool,
    pub phone_port: u16,
    /// Relay that devices away from home connect through, e.g.
    /// `https://relay.example.org`. None keeps access to the local network.
    pub relay_url: Option<String>,
    /// How devices away from home reach this computer, besides the relay.
    pub remote_access: RemoteAccess,
    /// Cloudflare tunnel token, for `RemoteAccess::Cloudflare`.
    pub tunnel_token: Option<String>,
    /// The stable address this computer is reachable at from anywhere, e.g.
    /// `https://ai.example.org`: the hostname of a Cloudflare tunnel, a
    /// Tailscale Funnel address or your own reverse proxy. Pairing links and
    /// QR codes use it.
    pub public_url: Option<String>,
    /// Use this cloudflared instead of downloading one.
    pub cloudflared_path: Option<PathBuf>,
    /// Look on GitHub for newer releases at startup.
    pub check_for_updates: bool,
    /// Model settings for every chat, from every device. A chat can still
    /// set its own; anything left out uses the model's defaults.
    pub chat_defaults: SamplingOptions,
    /// Community skill index to browse. None uses the BrainWashed registry.
    pub skill_index: Option<String>,
}

/// Ways to reach the host from outside the local network with no server of
/// your own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteAccess {
    /// Local network only (plus the relay, when one is set).
    Off,
    /// A free Cloudflare quick tunnel: no account, a new
    /// `https://<random>.trycloudflare.com` address each time it starts.
    #[default]
    Quick,
    /// A Cloudflare tunnel on your own domain, from `tunnel_token`. The
    /// address stays the same; set it as `public_url`.
    Cloudflare,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            host_name: None,
            backend: Backend::Auto,
            context_size: 8192,
            gpu_layers: 999,
            llama_server_path: None,
            active_model: None,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            disabled_skills: Vec::new(),
            phone_access: false,
            phone_port: 47860,
            relay_url: None,
            remote_access: RemoteAccess::default(),
            tunnel_token: None,
            public_url: None,
            cloudflared_path: None,
            check_for_updates: true,
            chat_defaults: SamplingOptions::default(),
            skill_index: None,
        }
    }
}
