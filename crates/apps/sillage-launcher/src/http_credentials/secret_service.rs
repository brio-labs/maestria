use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zbus::zvariant::{Dict, OwnedObjectPath, OwnedValue, Signature, Str, Type, Value};
use zbus::{Connection, Proxy};
use zeroize::Zeroize;

use super::{HttpGrantError, SecretBytes};

const SERVICE_NAME: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_PATH: &str = "/org/freedesktop/secrets/aliases/default";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const PROMPT_INTERFACE: &str = "org.freedesktop.Secret.Prompt";
const APP_ATTRIBUTE: &str = "io.github.briolabs.sillage.application";
const KIND_ATTRIBUTE: &str = "io.github.briolabs.sillage.credential-kind";
const HANDLE_ATTRIBUTE: &str = "io.github.briolabs.sillage.grant-handle";
const APP_ID: &str = "io.github.briolabs.sillage-launcher";
const CREDENTIAL_KIND: &str = "http-bearer-v1";
const SECRET_LABEL: &str = "Sillage HTTP bearer grant";
const DBUS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Serialize, Type)]
struct SecretToStore<'a> {
    session: &'a OwnedObjectPath,
    parameters: &'a [u8],
    value: &'a [u8],
    content_type: &'a str,
}

#[derive(Deserialize, Type)]
struct RetrievedSecret {
    session: OwnedObjectPath,
    parameters: Vec<u8>,
    value: Vec<u8>,
    content_type: String,
}

pub(super) async fn store(handle: &str, secret: &SecretBytes) -> Result<(), HttpGrantError> {
    let (connection, session, result) =
        tokio::time::timeout(DBUS_TIMEOUT, store_inner(handle, secret))
            .await
            .map_err(|_| HttpGrantError::Unavailable)??;
    close_session(&connection, &session).await;
    result
}

async fn store_inner(
    handle: &str,
    secret: &SecretBytes,
) -> Result<(Connection, OwnedObjectPath, Result<(), HttpGrantError>), HttpGrantError> {
    let connection = Connection::session()
        .await
        .map_err(|_| HttpGrantError::Unavailable)?;
    let session = open_session(&connection).await?;
    let result = async {
        let service = service_proxy(&connection).await?;
        let (unlocked, locked) = search(&service, handle).await?;
        if !unlocked.is_empty() || !locked.is_empty() {
            return Err(HttpGrantError::Failed);
        }

        let collection = Proxy::new(
            &connection,
            SERVICE_NAME,
            COLLECTION_PATH,
            COLLECTION_INTERFACE,
        )
        .await
        .map_err(|_| HttpGrantError::Unavailable)?;
        let locked: bool = collection
            .get_property("Locked")
            .await
            .map_err(|_| HttpGrantError::Unavailable)?;
        if locked {
            return Err(HttpGrantError::Unavailable);
        }
        let properties = item_properties(handle)?;
        let secret_value = SecretToStore {
            session: &session,
            parameters: &[],
            value: secret.as_bytes(),
            content_type: "application/octet-stream",
        };
        let (item, prompt) = collection
            .call::<_, _, (OwnedObjectPath, OwnedObjectPath)>(
                "CreateItem",
                &(&properties, &secret_value, false),
            )
            .await
            .map_err(|_| HttpGrantError::Unavailable)?;
        if item.as_str() == "/" {
            dismiss_prompt(&connection, &prompt).await;
            return Err(HttpGrantError::Unavailable);
        }
        if prompt.as_str() != "/" {
            dismiss_prompt(&connection, &prompt).await;
            return Err(HttpGrantError::Unavailable);
        }
        Ok(())
    }
    .await;
    Ok((connection, session, result))
}

pub(super) async fn resolve(handle: &str) -> Result<SecretBytes, HttpGrantError> {
    let (connection, session, result) = tokio::time::timeout(DBUS_TIMEOUT, resolve_inner(handle))
        .await
        .map_err(|_| HttpGrantError::Unavailable)??;
    close_session(&connection, &session).await;
    result
}

async fn resolve_inner(
    handle: &str,
) -> Result<
    (
        Connection,
        OwnedObjectPath,
        Result<SecretBytes, HttpGrantError>,
    ),
    HttpGrantError,
> {
    let connection = Connection::session()
        .await
        .map_err(|_| HttpGrantError::Unavailable)?;
    let session = open_session(&connection).await?;
    let result = async {
        let service = service_proxy(&connection).await?;
        let (unlocked, locked) = search(&service, handle).await?;
        if !locked.is_empty() || unlocked.len() != 1 {
            return Err(HttpGrantError::Unavailable);
        }
        let item = unlocked
            .into_iter()
            .next()
            .ok_or(HttpGrantError::Unavailable)?;
        let item_proxy = Proxy::new(&connection, SERVICE_NAME, item.as_str(), ITEM_INTERFACE)
            .await
            .map_err(|_| HttpGrantError::Unavailable)?;
        let mut secret: RetrievedSecret = item_proxy
            .call("GetSecret", &session)
            .await
            .map_err(|_| HttpGrantError::Unavailable)?;
        if secret.session != session
            || !secret.parameters.is_empty()
            || !matches!(
                secret.content_type.as_str(),
                "application/octet-stream" | "text/plain"
            )
        {
            secret.value.zeroize();
            return Err(HttpGrantError::Failed);
        }
        if super::policy::validate_secret(&secret.value).is_err() {
            secret.value.zeroize();
            return Err(HttpGrantError::Failed);
        }
        Ok(SecretBytes::new(secret.value))
    }
    .await;
    Ok((connection, session, result))
}

pub(super) async fn delete(handle: &str) -> Result<(), HttpGrantError> {
    let (connection, session, result) = tokio::time::timeout(DBUS_TIMEOUT, delete_inner(handle))
        .await
        .map_err(|_| HttpGrantError::Unavailable)??;
    close_session(&connection, &session).await;
    result
}

async fn delete_inner(
    handle: &str,
) -> Result<(Connection, OwnedObjectPath, Result<(), HttpGrantError>), HttpGrantError> {
    let connection = Connection::session()
        .await
        .map_err(|_| HttpGrantError::Unavailable)?;
    let session = open_session(&connection).await?;
    let result = async {
        let service = service_proxy(&connection).await?;
        let (unlocked, locked) = search(&service, handle).await?;
        if !locked.is_empty() || unlocked.len() > 1 {
            return Err(HttpGrantError::Unavailable);
        }
        let Some(item) = unlocked.into_iter().next() else {
            return Ok(());
        };
        let proxy = Proxy::new(&connection, SERVICE_NAME, item.as_str(), ITEM_INTERFACE)
            .await
            .map_err(|_| HttpGrantError::Unavailable)?;
        let prompt: OwnedObjectPath = proxy
            .call("Delete", &())
            .await
            .map_err(|_| HttpGrantError::Unavailable)?;
        if prompt.as_str() != "/" {
            dismiss_prompt(&connection, &prompt).await;
            return Err(HttpGrantError::Unavailable);
        }
        Ok(())
    }
    .await;
    Ok((connection, session, result))
}

async fn open_session(connection: &Connection) -> Result<OwnedObjectPath, HttpGrantError> {
    let service = service_proxy(connection).await?;
    let input = Value::new(Str::from(String::new()));
    let (_output, session): (OwnedValue, OwnedObjectPath) = service
        .call("OpenSession", &("plain", input))
        .await
        .map_err(|_| HttpGrantError::Unavailable)?;
    Ok(session)
}

async fn close_session(connection: &Connection, session: &OwnedObjectPath) {
    let close = async {
        if let Ok(proxy) = Proxy::new(
            connection,
            SERVICE_NAME,
            session.as_str(),
            "org.freedesktop.Secret.Session",
        )
        .await
        {
            let _: zbus::Result<()> = proxy.call("Close", &()).await;
        }
    };
    let _ = tokio::time::timeout(Duration::from_millis(250), close).await;
}

async fn service_proxy(connection: &Connection) -> Result<Proxy<'_>, HttpGrantError> {
    Proxy::new(connection, SERVICE_NAME, SERVICE_PATH, SERVICE_INTERFACE)
        .await
        .map_err(|_| HttpGrantError::Unavailable)
}

async fn search(
    service: &Proxy<'_>,
    handle: &str,
) -> Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>), HttpGrantError> {
    service
        .call("SearchItems", &attributes(handle))
        .await
        .map_err(|_| HttpGrantError::Unavailable)
}

fn attributes(handle: &str) -> BTreeMap<&str, &str> {
    BTreeMap::from([
        (APP_ATTRIBUTE, APP_ID),
        (KIND_ATTRIBUTE, CREDENTIAL_KIND),
        (HANDLE_ATTRIBUTE, handle),
    ])
}

fn item_properties(handle: &str) -> Result<BTreeMap<String, OwnedValue>, HttpGrantError> {
    let mut properties = BTreeMap::new();
    properties.insert(
        "org.freedesktop.Secret.Item.Label".to_owned(),
        variant(SECRET_LABEL)?,
    );
    let mut attributes = Dict::new(&Signature::Str, &Signature::Str);
    for (key, value) in [
        (APP_ATTRIBUTE, APP_ID),
        (KIND_ATTRIBUTE, CREDENTIAL_KIND),
        (HANDLE_ATTRIBUTE, handle),
    ] {
        attributes
            .add(key, value)
            .map_err(|_| HttpGrantError::Failed)?;
    }
    let attributes =
        OwnedValue::try_from(Value::from(attributes)).map_err(|_| HttpGrantError::Failed)?;
    properties.insert(
        "org.freedesktop.Secret.Item.Attributes".to_owned(),
        attributes,
    );
    Ok(properties)
}

fn variant(value: &str) -> Result<OwnedValue, HttpGrantError> {
    OwnedValue::try_from(Value::from(value.to_owned())).map_err(|_| HttpGrantError::Failed)
}

async fn dismiss_prompt(connection: &Connection, prompt: &OwnedObjectPath) {
    if prompt.as_str() == "/" {
        return;
    }
    if let Ok(proxy) = Proxy::new(connection, SERVICE_NAME, prompt.as_str(), PROMPT_INTERFACE).await
    {
        let _: zbus::Result<()> = proxy.call("Dismiss", &()).await;
    }
}
