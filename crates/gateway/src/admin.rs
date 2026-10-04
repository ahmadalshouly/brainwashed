//! The calls paired devices make after pairing. Members can chat and see
//! what's running; admins can also manage models, skills, settings, devices
//! and remote access, which is what the web admin page uses. Every change an
//! admin makes is written to the audit log.

use crate::devices::{Device, DeviceRole};
use crate::relay;
use crate::server::{download_entry, Gateway};
use brainwashed_core::{Provider, ProviderInfo, Settings};
use serde_json::{json, Value};

/// Calls any paired device may make.
const MEMBER_METHODS: &[&str] = &[
    "info",
    "state",
    "models",
    "skills",
    "whoami",
    "readDocument",
    "chatModels",
    "chatDefaults",
];

/// Calls that change something, recorded in the audit log.
const AUDITED: &[&str] = &[
    "loadModel",
    "unloadModel",
    "downloadModel",
    "deleteModel",
    "setSkillEnabled",
    "saveSkill",
    "deleteSkill",
    "installSkill",
    "updateSettings",
    "createPairingOffer",
    "removeDevice",
    "setDeviceRole",
    "renameDevice",
    "saveProvider",
    "deleteProvider",
];

type CallResult = Result<Value, String>;

fn to_json<T: serde::Serialize>(v: T) -> CallResult {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn str_param<'a>(params: &'a Value, name: &str) -> Result<&'a str, String> {
    params[name]
        .as_str()
        .ok_or_else(|| format!("missing {name}"))
}

fn role_param(params: &Value) -> Result<DeviceRole, String> {
    serde_json::from_value(params["role"].clone())
        .map_err(|_| "role must be admin or member".into())
}

pub(crate) async fn handle(
    gw: &Gateway,
    device: &Device,
    method: &str,
    params: Value,
) -> CallResult {
    if device.role != DeviceRole::Admin && !MEMBER_METHODS.contains(&method) {
        return Err("Only admins can do that. Ask an admin to make this device an admin.".into());
    }
    let target = ["id", "name", "repo", "role", "spec"]
        .iter()
        .find_map(|k| params[*k].as_str().or(params["provider"][*k].as_str()))
        .map(str::to_string);
    let result = call(gw, device, method, params).await;
    if AUDITED.contains(&method) {
        gw.audit(
            Some(device),
            method,
            target.as_deref(),
            result.as_ref().err().map(String::as_str),
        );
    }
    result
}

async fn call(gw: &Gateway, device: &Device, method: &str, params: Value) -> CallResult {
    let engine = &gw.inner.engine;
    let err = |e: brainwashed_core::Error| e.to_string();
    match method {
        // ----- everyone -----
        "info" => to_json(engine.info()),
        "state" => to_json(engine.state()),
        "models" => to_json(engine.models()),
        "skills" => to_json(engine.skills().skills),
        "whoami" => Ok(json!({
            "deviceId": device.id,
            "name": device.name,
            "role": device.role,
        })),

        "chatModels" => to_json(engine.chat_models(device.role == DeviceRole::Admin)),
        "chatDefaults" => to_json(engine.settings().chat_defaults),
        "readDocument" => {
            use base64::Engine as _;
            let name = str_param(&params, "name")?.to_string();
            let data = str_param(&params, "data")?;
            if data.len() > brainwashed_core::MAX_DOCUMENT_BYTES / 3 * 4 + 4 {
                return Err(format!(
                    "{name} is too big. Attach files up to {} MB.",
                    brainwashed_core::MAX_DOCUMENT_BYTES / 1024 / 1024
                ));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| "the file didn't arrive intact".to_string())?;
            // PDFs can take a moment; keep it off the async threads.
            tokio::task::spawn_blocking(move || brainwashed_core::read_document(&name, &bytes))
                .await
                .map_err(|_| "reading the file failed".to_string())?
                .map_err(err)
                .and_then(to_json)
        }

        // ----- models -----
        "hardware" => to_json(engine.hardware()),
        "catalog" => to_json(engine.catalog()),
        "loadModel" => {
            let id = str_param(&params, "id")?.to_string();
            if !engine.models().iter().any(|m| m.id == id) {
                return Err(format!("no installed model `{id}`"));
            }
            // Loading can take minutes; reply now and let the device poll state.
            let engine = engine.clone();
            tokio::spawn(async move {
                if let Err(e) = engine.load_model(&id).await {
                    tracing::warn!("loading {id} failed: {e}");
                }
            });
            Ok(Value::Null)
        }
        "unloadModel" => {
            engine.unload().await.map_err(err)?;
            Ok(Value::Null)
        }
        "downloadModel" => {
            let repo = str_param(&params, "repo")?.trim().to_string();
            let quant = params["quant"].as_str().map(str::to_string);
            {
                let mut downloads = gw.inner.downloads.lock().unwrap();
                if downloads.iter().any(|d| d.repo == repo && !d.finished) {
                    return Err(format!("{repo} is already downloading"));
                }
                *download_entry(&mut downloads, &repo) = crate::server::DownloadStatus {
                    repo: repo.clone(),
                    done: 0,
                    total: None,
                    finished: false,
                    error: None,
                };
            }
            // Downloads take a while; the admin page polls `downloads`.
            let engine = engine.clone();
            tokio::spawn(async move {
                if let Err(e) = engine.download_model(&repo, quant.as_deref()).await {
                    tracing::warn!("downloading {repo} failed: {e}");
                }
            });
            Ok(Value::Null)
        }
        "downloads" => to_json(gw.inner.downloads.lock().unwrap().clone()),
        "deleteModel" => {
            engine
                .delete_model(str_param(&params, "id")?)
                .await
                .map_err(err)?;
            Ok(Value::Null)
        }

        // ----- cloud providers -----
        "providers" => to_json(
            engine
                .providers()
                .iter()
                .map(ProviderInfo::from)
                .collect::<Vec<_>>(),
        ),
        "saveProvider" => {
            let provider: Provider = serde_json::from_value(params["provider"].clone())
                .map_err(|e| format!("bad provider: {e}"))?;
            engine
                .save_provider(provider)
                .map_err(err)
                .and_then(|p| to_json(ProviderInfo::from(&p)))
        }
        "deleteProvider" => {
            engine
                .delete_provider(str_param(&params, "id")?)
                .map_err(err)?;
            Ok(Value::Null)
        }
        "providerModels" => engine
            .provider_models(
                str_param(&params, "baseUrl")?,
                params["apiKey"].as_str(),
                params["id"].as_str(),
            )
            .await
            .map_err(err)
            .and_then(to_json),

        // ----- skills -----
        "skillList" => to_json(engine.skills()),
        "skillSource" => engine
            .skill_source(str_param(&params, "name")?)
            .map_err(err)
            .map(Value::String),
        "saveSkill" => {
            let source = str_param(&params, "source")?;
            let previous = params["previousName"].as_str();
            engine
                .save_skill(source, previous)
                .map_err(err)
                .map(Value::String)
        }
        "deleteSkill" => {
            engine
                .delete_skill(str_param(&params, "name")?)
                .map_err(err)?;
            Ok(Value::Null)
        }
        "communitySkills" => engine
            .community_skills()
            .await
            .map_err(err)
            .and_then(to_json),
        "previewSkill" => engine
            .preview_skill(str_param(&params, "spec")?)
            .await
            .map_err(err)
            .and_then(to_json),
        "installSkill" => engine
            .install_skill(
                str_param(&params, "spec")?,
                params["sha256"].as_str(),
                params["replace"].as_bool().unwrap_or(false),
            )
            .await
            .map_err(err)
            .and_then(to_json),
        "setSkillEnabled" => {
            let name = str_param(&params, "name")?;
            let enabled = params["enabled"].as_bool().ok_or("missing enabled")?;
            engine.set_skill_enabled(name, enabled).map_err(err)?;
            Ok(Value::Null)
        }

        // ----- settings and remote access -----
        "settings" => Ok(public_settings(&engine.settings())),
        "updateSettings" => {
            let settings = merge_settings(engine.settings(), &params["settings"])?;
            engine.update_settings(settings).map_err(err)?;
            gw.apply_settings().map_err(|e| e.to_string())?;
            Ok(public_settings(&engine.settings()))
        }
        "access" => to_json(gw.status()),
        "checkForUpdate" => to_json(engine.check_for_update().await),

        // ----- devices -----
        "devices" => {
            let list: Vec<Value> = gw
                .devices()
                .into_iter()
                .map(|d| {
                    let mut v = serde_json::to_value(&d).unwrap_or_default();
                    v["current"] = json!(d.id == device.id);
                    v
                })
                .collect();
            Ok(Value::Array(list))
        }
        "createPairingOffer" => {
            let role = if params["role"].is_null() {
                DeviceRole::Member
            } else {
                role_param(&params)?
            };
            let offer = gw.create_pairing_offer(role).map_err(|e| e.to_string())?;
            let mut v = serde_json::to_value(&offer).map_err(|e| e.to_string())?;
            v["qr"] = json!(qr_rows(&offer.web_url));
            Ok(v)
        }
        "removeDevice" => {
            gw.remove_device(str_param(&params, "id")?)
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "setDeviceRole" => {
            let id = str_param(&params, "id")?;
            let role = role_param(&params)?;
            if role == DeviceRole::Member {
                let admins = gw
                    .devices()
                    .iter()
                    .filter(|d| d.role == DeviceRole::Admin && d.id != id)
                    .count();
                if admins == 0 {
                    return Err("Keep at least one admin.".into());
                }
            }
            if !gw
                .inner
                .devices
                .set_role(id, role)
                .map_err(|e| e.to_string())?
            {
                return Err("no such device".into());
            }
            gw.devices_changed();
            Ok(Value::Null)
        }
        "renameDevice" => {
            let name = str_param(&params, "name")?.trim();
            if name.is_empty() {
                return Err("give the device a name".into());
            }
            if !gw
                .inner
                .devices
                .rename(str_param(&params, "id")?, name)
                .map_err(|e| e.to_string())?
            {
                return Err("no such device".into());
            }
            gw.devices_changed();
            Ok(Value::Null)
        }
        "auditLog" => {
            let limit = params["limit"].as_u64().unwrap_or(100).min(500) as usize;
            to_json(gw.inner.audit.recent(limit))
        }

        other => Err(format!("unknown method `{other}`")),
    }
}

/// The QR code for a link as rows of `1` (dark) and `0` (light), so the web
/// page can draw it without a QR library.
fn qr_rows(text: &str) -> Vec<String> {
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
        return Vec::new();
    };
    let width = code.width();
    code.to_colors()
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' })
                .collect()
        })
        .collect()
}

/// Settings as admins see them: the tunnel token is never sent back, only
/// whether one is set.
fn public_settings(settings: &Settings) -> Value {
    let mut v = serde_json::to_value(settings).unwrap_or_default();
    v["tunnel_token_set"] = json!(settings
        .tunnel_token
        .as_deref()
        .is_some_and(|t| !t.is_empty()));
    v["tunnel_token"] = Value::Null;
    v
}

/// Applies the fields in `patch` to `settings`. Fields left out keep their
/// value, so the tunnel token survives a save that doesn't mention it.
fn merge_settings(settings: Settings, patch: &Value) -> Result<Settings, String> {
    let patch = patch.as_object().ok_or("missing settings")?;
    let mut current = serde_json::to_value(&settings).map_err(|e| e.to_string())?;
    for (key, value) in patch {
        if key == "tunnel_token_set" {
            continue;
        }
        if current.get(key).is_none() {
            return Err(format!("unknown setting `{key}`"));
        }
        current[key] = value.clone();
    }
    let mut next: Settings =
        serde_json::from_value(current).map_err(|e| format!("bad settings: {e}"))?;
    for url in [&mut next.relay_url, &mut next.public_url] {
        *url = match url.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(u) => {
                Some(relay::normalize_url(u).map_err(|e| e.replace("relay address", "address"))?)
            }
        };
    }
    next.skill_index = match next.skill_index.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(u) if u.starts_with("https://") || u.starts_with("http://localhost") => {
            Some(u.to_string())
        }
        Some(_) => return Err("the skills index address must start with https://".into()),
    };
    if next
        .tunnel_token
        .as_deref()
        .is_some_and(|t| t.trim().is_empty())
    {
        next.tunnel_token = None;
    }
    if next.context_size < 512 {
        return Err("context size must be at least 512".into());
    }
    if next.system_prompt.len() > 20_000 {
        return Err("the system prompt is too long".into());
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brainwashed_core::RemoteAccess;

    #[test]
    fn merges_only_what_changed() {
        let s = Settings {
            tunnel_token: Some("secret".into()),
            ..Default::default()
        };
        let next = merge_settings(
            s,
            &json!({ "context_size": 4096, "public_url": " https://ai.example.org/ ", "tunnel_token_set": true }),
        )
        .unwrap();
        assert_eq!(next.context_size, 4096);
        assert_eq!(next.public_url.as_deref(), Some("https://ai.example.org"));
        assert_eq!(next.tunnel_token.as_deref(), Some("secret"));

        let cleared =
            merge_settings(next, &json!({ "tunnel_token": "", "remote_access": "off" })).unwrap();
        assert_eq!(cleared.tunnel_token, None);
        assert_eq!(cleared.remote_access, RemoteAccess::Off);
    }

    #[test]
    fn refuses_bad_settings() {
        assert!(merge_settings(Settings::default(), &json!({ "nope": 1 })).is_err());
        assert!(merge_settings(
            Settings::default(),
            &json!({ "public_url": "ai.example.org" })
        )
        .is_err());
        assert!(merge_settings(Settings::default(), &json!({ "context_size": 10 })).is_err());
    }

    #[test]
    fn draws_qr_codes_as_rows() {
        let rows = qr_rows("https://example.org/#pair?v=1");
        assert!(rows.len() >= 21);
        assert!(rows.iter().all(|r| r.len() == rows.len()));
        assert!(rows[0].starts_with("1111111"));
    }

    #[test]
    fn never_shows_the_tunnel_token() {
        let s = Settings {
            tunnel_token: Some("secret".into()),
            ..Default::default()
        };
        let v = public_settings(&s);
        assert_eq!(v["tunnel_token"], Value::Null);
        assert_eq!(v["tunnel_token_set"], json!(true));
    }
}
