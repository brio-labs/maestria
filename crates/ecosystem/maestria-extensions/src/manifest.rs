use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

pub const SDK_API_VERSION: u32 = 1;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub api_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub entrypoints: Vec<Entrypoint>,
    pub commands: Vec<Command>,
    pub permissions: Vec<Permission>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entrypoint {
    pub id: String,
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub entrypoint_id: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Permission {
    #[serde(rename = "fileSearch")]
    FileSearch {
        #[serde(rename = "maxResults", default)]
        max_results: Option<u16>,
    },
    #[serde(rename = "userFileRead")]
    UserFileRead,
    #[serde(rename = "http")]
    Http { origins: Vec<String> },
    #[serde(rename = "storage")]
    Storage { scope: StorageScope },
    #[serde(rename = "notification")]
    Notification,
    #[serde(rename = "open")]
    Open { targets: Vec<OpenTarget> },
    #[serde(rename = "copy")]
    Copy { formats: Vec<CopyFormat> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageScope {
    Extension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpenTarget {
    Url,
    SelectedFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CopyFormat {
    Text,
}

impl Permission {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::FileSearch { .. } => "fileSearch",
            Self::UserFileRead => "userFileRead",
            Self::Http { .. } => "http",
            Self::Storage { .. } => "storage",
            Self::Notification => "notification",
            Self::Open { .. } => "open",
            Self::Copy { .. } => "copy",
        }
    }
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("extension manifest exceeds {MAX_MANIFEST_BYTES} bytes")]
    Oversized,
    #[error("invalid extension manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported SDK API version {0}; expected {SDK_API_VERSION}")]
    ApiVersion(u32),
    #[error("invalid {field}: {reason}")]
    Invalid { field: String, reason: &'static str },
    #[error("duplicate {field} {id:?}")]
    Duplicate { field: &'static str, id: String },
    #[error("command {command:?} declares missing entrypoint {entrypoint:?}")]
    MissingEntrypoint { command: String, entrypoint: String },
}

fn invalid(field: impl Into<String>, reason: &'static str) -> ManifestError {
    ManifestError::Invalid {
        field: field.into(),
        reason,
    }
}

fn valid_extension_id(id: &str) -> bool {
    id.len() <= 128
        && id.split(['.', '-']).all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
        && id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
}

fn valid_command_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn valid_text(text: &str, max: usize) -> bool {
    !text.is_empty()
        && text.encode_utf16().take(max + 1).count() <= max
        && text.trim() == text
        && !text.chars().any(char::is_control)
}

fn valid_relative_js(path: &str) -> bool {
    path.len() <= 240
        && path.ends_with(".js")
        && !path.contains(['\\', ':', '%'])
        && path.split('/').take(17).count() <= 16
        && path.split('/').all(|segment| {
            segment
                .as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
                && !matches!(segment, "." | "..")
        })
}

fn valid_https_origin(origin: &str) -> bool {
    if origin.len() > 255 {
        return false;
    }
    let Some(authority) = origin.strip_prefix("https://") else {
        return false;
    };
    let (host, port) = authority
        .rsplit_once(':')
        .map_or((authority, None), |(host, port)| (host, Some(port)));
    if host.len() > 253
        || host.is_empty()
        || !host.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
        || port.is_some_and(|port| {
            port.is_empty()
                || port.len() > 5
                || port.starts_with('0')
                || port.parse::<u16>().is_err()
        })
    {
        return false;
    }
    Url::parse(origin).is_ok_and(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::Oversized);
        }
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        self.validate_identity_and_limits()?;
        let entry_ids = self.validate_entrypoints()?;
        self.validate_commands(&entry_ids)?;
        self.validate_permissions()
    }

    fn validate_identity_and_limits(&self) -> Result<(), ManifestError> {
        if self.api_version != SDK_API_VERSION {
            return Err(ManifestError::ApiVersion(self.api_version));
        }
        if !valid_extension_id(&self.id) {
            return Err(invalid(
                "id",
                "expected a lowercase dotted/hyphenated path-independent ID",
            ));
        }
        if !valid_text(&self.name, 80) {
            return Err(invalid(
                "name",
                "expected trimmed, printable text of at most 80 characters",
            ));
        }
        if self.version.len() > 64 || semver::Version::parse(&self.version).is_err() {
            return Err(invalid(
                "version",
                "expected a semantic version of at most 64 characters",
            ));
        }
        if self.entrypoints.is_empty() || self.entrypoints.len() > 64 {
            return Err(invalid(
                "entrypoints",
                "expected 1–64 declared JavaScript files",
            ));
        }
        if self.commands.is_empty() || self.commands.len() > 128 {
            return Err(invalid("commands", "expected 1–128 declared commands"));
        }
        if self.permissions.len() > 7 {
            return Err(invalid(
                "permissions",
                "at most seven permission types are supported",
            ));
        }
        Ok(())
    }

    fn validate_entrypoints(&self) -> Result<BTreeSet<&str>, ManifestError> {
        let mut entry_ids = BTreeSet::new();
        let mut entry_files = BTreeSet::new();
        for (index, entry) in self.entrypoints.iter().enumerate() {
            if !valid_command_id(&entry.id) {
                return Err(invalid(
                    format!("entrypoints[{index}].id"),
                    "expected a lowercase ID of at most 48 characters",
                ));
            }
            if !valid_relative_js(&entry.file) {
                return Err(invalid(
                    format!("entrypoints[{index}].file"),
                    "expected a relative .js file without traversal",
                ));
            }
            if !entry_ids.insert(entry.id.as_str()) {
                return Err(ManifestError::Duplicate {
                    field: "entrypoint ID",
                    id: entry.id.clone(),
                });
            }
            if !entry_files.insert(entry.file.as_str()) {
                return Err(ManifestError::Duplicate {
                    field: "entrypoint file",
                    id: entry.file.clone(),
                });
            }
        }
        Ok(entry_ids)
    }

    fn validate_commands(&self, entry_ids: &BTreeSet<&str>) -> Result<(), ManifestError> {
        let mut command_ids = BTreeSet::new();
        for (index, command) in self.commands.iter().enumerate() {
            if !valid_command_id(&command.id) {
                return Err(invalid(
                    format!("commands[{index}].id"),
                    "expected a lowercase ID of at most 48 characters",
                ));
            }
            if !valid_text(&command.title, 80) {
                return Err(invalid(
                    format!("commands[{index}].title"),
                    "expected trimmed printable text of at most 80 characters",
                ));
            }
            if command
                .description
                .as_ref()
                .is_some_and(|description| !valid_text(description, 240))
            {
                return Err(invalid(
                    format!("commands[{index}].description"),
                    "expected trimmed printable text of at most 240 characters",
                ));
            }
            if !command_ids.insert(command.id.as_str()) {
                return Err(ManifestError::Duplicate {
                    field: "command ID",
                    id: command.id.clone(),
                });
            }
            if !entry_ids.contains(command.entrypoint_id.as_str()) {
                return Err(ManifestError::MissingEntrypoint {
                    command: command.id.clone(),
                    entrypoint: command.entrypoint_id.clone(),
                });
            }
        }
        Ok(())
    }

    fn validate_permissions(&self) -> Result<(), ManifestError> {
        let mut permission_kinds = BTreeSet::new();
        for permission in &self.permissions {
            if !permission_kinds.insert(permission.kind()) {
                return Err(ManifestError::Duplicate {
                    field: "permission",
                    id: permission.kind().to_owned(),
                });
            }
            validate_permission(permission)?;
        }
        Ok(())
    }
}

fn validate_permission(permission: &Permission) -> Result<(), ManifestError> {
    match permission {
        Permission::FileSearch {
            max_results: Some(0 | 101..),
        } => Err(invalid(
            "permissions.fileSearch.maxResults",
            "expected 1–100",
        )),
        Permission::Http { origins } => validate_http_origins(origins),
        Permission::Open { targets }
            if targets.is_empty()
                || targets.len() > 2
                || (targets.len() == 2 && targets[0] == targets[1]) =>
        {
            Err(invalid(
                "permissions.open.targets",
                "expected one or two distinct URL/selectedFile targets",
            ))
        }
        Permission::Copy { formats } if formats != &[CopyFormat::Text] => Err(invalid(
            "permissions.copy.formats",
            "only [\"text\"] is supported",
        )),
        _ => Ok(()),
    }
}

fn validate_http_origins(origins: &[String]) -> Result<(), ManifestError> {
    if origins.is_empty() || origins.len() > 32 {
        return Err(invalid(
            "permissions.http.origins",
            "expected 1–32 granted HTTPS origins",
        ));
    }
    let mut seen = BTreeSet::new();
    for origin in origins {
        if !valid_https_origin(origin) {
            return Err(invalid(
                "permissions.http.origins",
                "expected an exact lowercase HTTPS origin without credentials, path, query or fragment",
            ));
        }
        if !seen.insert(origin) {
            return Err(ManifestError::Duplicate {
                field: "HTTP origin",
                id: origin.clone(),
            });
        }
    }
    Ok(())
}
