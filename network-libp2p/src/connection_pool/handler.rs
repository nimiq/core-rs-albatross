use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
    task::{Context, Poll},
    time::{Duration, SystemTime},
};

use ip_network::IpNetwork;
use libp2p::swarm::{
    protocols_handler::{InboundUpgradeSend, OutboundUpgradeSend},
    KeepAlive, ProtocolsHandler, ProtocolsHandlerEvent, ProtocolsHandlerUpgrErr, SubstreamProtocol,
};
use thiserror::Error;

use super::protocol::ConnectionPoolProtocol;

#[derive(Clone, Debug)]
pub enum HandlerInEvent {
    Ban { ip: IpNetwork },
}

#[derive(Clone, Debug)]
pub enum HandlerOutEvent {
    Banned { ip: IpNetwork },
    Unbanned { ip: IpNetwork },
}

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("ConnectionPoolBehaviour error")]
    ConnectionPoolError,
}

pub struct ConnectionPoolHandler {
    banned: HashMap<IpNetwork, SystemTime>,
    ip_ban: VecDeque<IpNetwork>,
}

impl ConnectionPoolHandler {
    pub fn new() -> Self {
        Self {
            banned: HashMap::new(),
            ip_ban: VecDeque::new(),
        }
    }
}

impl ProtocolsHandler for ConnectionPoolHandler {
    type InEvent = HandlerInEvent;
    type OutEvent = HandlerOutEvent;
    type Error = HandlerError;
    type InboundProtocol = ConnectionPoolProtocol;
    type OutboundProtocol = ConnectionPoolProtocol;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<ConnectionPoolProtocol, ()> {
        SubstreamProtocol::new(ConnectionPoolProtocol, ())
    }

    fn inject_fully_negotiated_inbound(
        &mut self,
        _protocol: <Self::InboundProtocol as InboundUpgradeSend>::Output,
        _info: Self::InboundOpenInfo,
    ) {
    }

    fn inject_fully_negotiated_outbound(
        &mut self,
        _protocol: <Self::OutboundProtocol as OutboundUpgradeSend>::Output,
        _info: Self::OutboundOpenInfo,
    ) {
    }

    fn inject_event(&mut self, event: Self::InEvent) {
        match event {
            HandlerInEvent::Ban { ip } => {
                self.banned
                    .insert(ip, SystemTime::now() + Duration::from_secs(60 * 10)); // Ban time: 10 minutes
                self.ip_ban.push_back(ip);
            }
        }
    }

    fn inject_dial_upgrade_error(
        &mut self,
        _info: Self::OutboundOpenInfo,
        _error: ProtocolsHandlerUpgrErr<<Self::OutboundProtocol as OutboundUpgradeSend>::Error>,
    ) {
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::No
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<
        ProtocolsHandlerEvent<
            Self::OutboundProtocol,
            Self::OutboundOpenInfo,
            Self::OutEvent,
            Self::Error,
        >,
    > {
        for (ip, time) in self.banned.clone() {
            if time < SystemTime::now() {
                self.banned.remove(&ip);
                return Poll::Ready(ProtocolsHandlerEvent::Custom(HandlerOutEvent::Unbanned {
                    ip,
                }));
            }
        }

        if let Some(ip) = self.ip_ban.pop_front() {
            return Poll::Ready(ProtocolsHandlerEvent::Custom(HandlerOutEvent::Banned {
                ip,
            }));
        }

        Poll::Pending
    }
}
