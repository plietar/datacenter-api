use crate::config::Config;
use crate::ipmi::{ChassisControl, GetChassisStatus, PowerRestorePolicy, ipmi_do};

use axum::Json;
use axum::extract::{Path, State};
use futures::FutureExt;
use futures::TryFutureExt;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostState {
    power_is_on: bool,
    power_restore_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Error {
    error: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HostCommand {
    power: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(transparent)]
struct Either<T, U>(#[serde(with = "either::serde_untagged")] either::Either<T, U>)
where
    T: Serialize + for<'a> Deserialize<'a>,
    U: Serialize + for<'a> Deserialize<'a>;

impl<T, U> Either<T, U>
where
    T: Serialize + for<'a> Deserialize<'a>,
    U: Serialize + for<'a> Deserialize<'a>,
{
    fn left(v: T) -> Either<T, U> {
        Either(either::Either::Left(v))
    }
    fn right(v: U) -> Either<T, U> {
        Either(either::Either::Right(v))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HostList {
    hosts: HashMap<String, Either<HostState, Error>>,
}

pub async fn ipmi_hosts_handler(State(config): State<Config>) -> Json<HostList> {
    let hosts = stream::iter(config.host)
        .map(|(hostname, host)| {
            ipmi_do(
                &host.address,
                &config.ipmi.username,
                config.ipmi.password.as_ref().unwrap().as_bytes(),
                GetChassisStatus,
            )
            .map_ok(move |response| HostState {
                power_is_on: response.power_is_on,
                power_restore_policy: match response.power_restore_policy {
                    PowerRestorePolicy::AlwaysOn => "always-on".to_owned(),
                    PowerRestorePolicy::AlwaysOff => "always-off".to_owned(),
                    PowerRestorePolicy::Previous => "previous".to_owned(),
                },
            })
            .map_err(|e| Error {
                error: format!("{:?}", e),
            })
            .map_ok_or_else(Either::right, Either::left)
            .map(move |v| (hostname, v))
        })
        .buffer_unordered(4)
        .collect()
        .await;

    Json(HostList { hosts })
}

pub async fn ipmi_host_put_handler(
    Path(hostname): Path<String>,
    State(config): State<Config>,
    Json(body): Json<HostCommand>,
) {
    let host = &config.host[&hostname];

    let cmd = match body.power {
        Some(true) => Some(ChassisControl::PowerUp),
        Some(false) => Some(ChassisControl::PowerDown),
        None => None,
    };

    if let Some(cmd) = cmd {
        ipmi_do(
            &host.address,
            &config.ipmi.username,
            config.ipmi.password.as_ref().unwrap().as_bytes(),
            cmd,
        )
        .await
        .unwrap();
    }
}
