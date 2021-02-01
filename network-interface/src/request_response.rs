use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::{
    channel::oneshot::{channel, Sender},
    future, StreamExt,
};
use parking_lot::Mutex;
use tokio::{task::spawn, time::timeout};
use thiserror::Error;

use crate::message::*;
use crate::peer::*;

struct RequestResponseState<Res: ResponseMessage> {
    current_request_identifier: u32,
    responses: HashMap<u32, Sender<Res>>,
}

pub type ResponseTimeout = u32;

pub struct RequestResponse<P: Peer, Req: RequestMessage, Res: ResponseMessage> {
    peer: Arc<P>,
    state: Arc<Mutex<RequestResponseState<Res>>>,
    timeout: Duration,
    _req_type: PhantomData<Req>,
}

#[derive(Debug, Error)]
pub enum RequestError {
    #[error("Timeout error")]
    Timeout,
    #[error("Send error: {0}")]
    SendError(#[from] SendError),
    #[error("Receive error")]
    ReceiveError,
}

// Probably not really `Message` as types, but something that has a request identifier.
impl<P: Peer, Req: RequestMessage, Res: ResponseMessage + 'static> RequestResponse<P, Req, Res> {
    pub fn new(peer: Arc<P>, timeout: Duration) -> Self {
        let state = Arc::new(Mutex::new(RequestResponseState {
            current_request_identifier: 0,
            responses: Default::default(),
        }));

        // Poll stream and distribute messages to oneshot channels.
        let stream = peer.receive::<Res>();
        let weak_state = Arc::downgrade(&state);
        let weak_state2 = Weak::clone(&weak_state);
        // We only poll the stream while this struct still exists (as indicated by the weak ref).
        spawn(
            stream
                .take_while(move |_: &Res| future::ready(weak_state2.strong_count() > 0))
                .for_each(move |item: Res| {
                    if let Some(state) = weak_state.upgrade() {
                        let request_identifier = item.get_request_identifier();
                        let mut state = state.lock();
                        if let Some(sender) = state.responses.remove(&request_identifier) {
                            sender.send(item).ok();
                        }
                    }
                    future::ready(())
                }),
        );

        RequestResponse {
            peer,
            state,
            timeout,
            _req_type: PhantomData,
        }
    }

    pub async fn request(&self, mut request: Req) -> Result<Res, RequestError> {
        log::debug!("Sending request: {:#?}", request);

        // Lock state, set identifier and send out request. Also add channel to the state.
        let (request_identifier, receiver) = {
            let mut state = self.state.lock();
            let request_identifier = state.current_request_identifier;
            state.current_request_identifier += 1;

            request.set_request_identifier(request_identifier);

            let (sender, receiver) = channel();
            state.responses.insert(request_identifier, sender);

            (request_identifier, receiver)
        };

        // TODO: CloseType
        // If sending fails, remove channel and return error.
        if let Err(e) = self.peer.send_or_close(&request, |_| CloseReason::Other).await {
            let mut state = self.state.lock();
            state.responses.remove(&request_identifier);
            return Err(RequestError::SendError(e));
        }

        // Now we only have to wait for the response.
        match timeout(self.timeout, receiver).await {
            Ok(Ok(response)) => {
                log::debug!("Received response: {:#?}", response);

                Ok(response)
            },
            Ok(Err(e)) => {
                log::error!("Receive error: {}", e);

                Err(RequestError::ReceiveError)
            },
            Err(_) => {
                log::error!("Timeout");

                // Lock state and remove channel on timeout.
                let mut state = self.state.lock();
                state.responses.remove(&request_identifier);
                Err(RequestError::Timeout)
            }
        }
    }
}
