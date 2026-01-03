use futures::TryFutureExt;
use ipmi_rs::connection::rmcp::Rmcp;
use ipmi_rs::connection::CompletionCode;
use ipmi_rs::connection::IpmiCommand;
use ipmi_rs::connection::Message;
use ipmi_rs::connection::NetFn;
use ipmi_rs::connection::ParseResponseError;
use ipmi_rs::Ipmi;
use std::time::Duration;

#[derive(Copy, Clone, Debug)]
#[allow(dead_code)]
pub enum PowerRestorePolicy {
    AlwaysOn,
    AlwaysOff,
    Previous,
}

#[derive(Copy, Clone, Debug)]
#[allow(dead_code)]
pub struct ChassisStatus {
    pub power_restore_policy: PowerRestorePolicy,
    pub power_control_fault: bool,
    pub power_fault: bool,
    pub interlock: bool,
    pub power_overload: bool,
    pub power_is_on: bool,

    pub last_power_on_by_command: bool,
    pub last_power_on_by_fault: bool,
    pub last_power_on_by_interlock_activated: bool,
    pub last_power_on_by_overload: bool,
    pub ac_failed: bool,
}

impl ChassisStatus {
    fn from_data(data: &[u8]) -> Option<ChassisStatus> {
        const POLICIES: [PowerRestorePolicy; 3] = [
            PowerRestorePolicy::AlwaysOff,
            PowerRestorePolicy::Previous,
            PowerRestorePolicy::AlwaysOn,
        ];
        if data.len() < 4 {
            return None;
        }
        Some(ChassisStatus {
            power_is_on: (data[0] & 1) != 0,
            power_overload: (data[0] & 2) != 0,
            interlock: (data[0] & 4) != 0,
            power_fault: (data[0] & 8) != 0,
            power_control_fault: (data[0] & 16) != 0,
            power_restore_policy: POLICIES[((data[0] >> 5) & 0b11) as usize],

            last_power_on_by_command: (data[1] & 16) != 0,
            last_power_on_by_fault: (data[1] & 8) != 0,
            last_power_on_by_interlock_activated: (data[1] & 4) != 0,
            last_power_on_by_overload: (data[1] & 2) != 0,
            ac_failed: (data[1] & 1) != 0,
        })
    }
}

pub struct GetChassisStatus;

impl Into<Message> for GetChassisStatus {
    fn into(self) -> Message {
        Message::new_request(NetFn::Chassis, 0x01, Vec::new())
    }
}

impl ipmi_rs::connection::IpmiCommand for GetChassisStatus {
    type Output = ChassisStatus;
    type Error = ();

    fn parse_response(
        completion_code: CompletionCode,
        data: &[u8],
    ) -> Result<Self::Output, ParseResponseError<Self::Error>> {
        Self::check_cc_success(completion_code)?;
        ChassisStatus::from_data(data).ok_or(ParseResponseError::NotEnoughData)
    }
}

#[derive(Copy, Clone, Debug)]
#[allow(dead_code)]
pub enum ChassisControl {
    PowerDown = 0,
    PowerUp = 1,
    PowerCycle = 2,
    HardReset = 3,
}

impl Into<Message> for ChassisControl {
    fn into(self) -> Message {
        Message::new_request(NetFn::Chassis, 0x02, vec![self as u8])
    }
}

impl IpmiCommand for ChassisControl {
    type Output = ();
    type Error = ();

    fn parse_response(
        completion_code: CompletionCode,
        _data: &[u8],
    ) -> Result<Self::Output, ParseResponseError<Self::Error>> {
        Self::check_cc_success(completion_code)?;
        Ok(())
    }
}

pub fn ipmi_do<C: IpmiCommand>(
    hostname: &str,
    username: &str,
    password: &[u8],
    cmd: C,
) -> impl Future<Output = anyhow::Result<C::Output>> + use<C>
where
    C: Send + 'static,
    C::Output: Send + 'static,
    C::Error: Send + Sync + 'static + std::fmt::Debug,
{
    let hostname = hostname.to_owned();
    let username = username.to_owned();
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut rmcp = Rmcp::new((hostname.as_ref(), 623), Duration::from_secs(5)).unwrap();
        rmcp.activate(true, Some(&username), Some(&password))
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;

        let mut ipmi = Ipmi::new(rmcp);
        let result = ipmi
            .send_recv(cmd)
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;
        Ok(result)
    })
    .unwrap_or_else(|e: tokio::task::JoinError| panic!("ipmi command panicked: {:?}", e))
}
