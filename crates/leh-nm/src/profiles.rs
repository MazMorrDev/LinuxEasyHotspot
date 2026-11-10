//! Finding, creating and updating the NM profiles the app owns.

use std::collections::HashMap;

use leh_model::{NmSettings, SettingValue};
use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::proxies::{SettingsConnectionProxy, SettingsDict, SettingsProxy};

const FLAG_TO_DISK: u32 = 0x1;

/// Converts the typed DTO into the D-Bus dictionary.
pub fn to_dict(s: &NmSettings) -> SettingsDict<'static> {
    s.0.iter()
        .map(|(setting, props)| {
            let props = props
                .iter()
                .map(|(k, v)| {
                    let v = match v {
                        SettingValue::Str(s) => Value::from(s.clone()),
                        SettingValue::Bool(b) => Value::from(*b),
                        SettingValue::U32(n) => Value::from(*n),
                        SettingValue::I32(n) => Value::from(*n),
                        SettingValue::Bytes(b) => Value::from(b.clone()),
                        SettingValue::StrList(l) => Value::from(l.clone()),
                    };
                    (k.clone(), v)
                })
                .collect();
            (setting.clone(), props)
        })
        .collect()
}

/// A saved profile: its D-Bus path and UUID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub path: OwnedObjectPath,
    pub uuid: String,
}

/// Finds a profile by UUID first, then by `connection.id`.
pub async fn find(
    conn: &Connection,
    uuid: Option<&str>,
    id: &str,
) -> zbus::Result<Option<Profile>> {
    let settings = SettingsProxy::new(conn).await?;
    let mut by_id = None;
    for path in settings.list_connections().await? {
        let c = SettingsConnectionProxy::builder(conn)
            .path(path.clone())?
            .build()
            .await?;
        let Ok(s) = c.get_settings().await else {
            continue; // not visible to this user
        };
        let Some(section) = s.get("connection") else {
            continue;
        };
        let get = |k: &str| {
            section
                .get(k)
                .and_then(|v| v.downcast_ref::<&str>().ok().map(str::to_owned))
        };
        let (Some(c_uuid), Some(c_id)) = (get("uuid"), get("id")) else {
            continue;
        };
        if uuid == Some(c_uuid.as_str()) {
            return Ok(Some(Profile { path, uuid: c_uuid }));
        }
        if c_id == id && by_id.is_none() {
            by_id = Some(Profile { path, uuid: c_uuid });
        }
    }
    Ok(by_id)
}

/// Updates `existing` in place, or adds a new persistent profile.
/// The settings' `connection.uuid` must already be the right one.
pub async fn upsert(
    conn: &Connection,
    settings: &NmSettings,
    existing: Option<&Profile>,
) -> zbus::Result<OwnedObjectPath> {
    tracing::debug!(settings = ?settings.redacted(), "writing NM profile");
    let dict = to_dict(settings);
    match existing {
        Some(p) => {
            let c = SettingsConnectionProxy::builder(conn)
                .path(p.path.clone())?
                .build()
                .await?;
            c.update2(dict, FLAG_TO_DISK, HashMap::new()).await?;
            Ok(p.path.clone())
        }
        None => {
            let s = SettingsProxy::new(conn).await?;
            let (path, _) = s
                .add_connection2(dict, FLAG_TO_DISK, HashMap::new())
                .await?;
            Ok(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dict_types() {
        let mut s = NmSettings::default();
        s.set("a", "s", SettingValue::str("x"))
            .set("a", "ay", SettingValue::Bytes(vec![1, 2]))
            .set("a", "as", SettingValue::str_list(["r"]))
            .set("a", "u", SettingValue::U32(4));
        let d = to_dict(&s);
        let sigs: HashMap<_, _> = d["a"]
            .iter()
            .map(|(k, v)| (k.as_str(), v.value_signature().to_string()))
            .collect();
        assert_eq!(sigs["s"], "s");
        assert_eq!(sigs["ay"], "ay");
        assert_eq!(sigs["as"], "as");
        assert_eq!(sigs["u"], "u");
    }
}
