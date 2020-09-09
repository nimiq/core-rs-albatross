use futures::{
    channel::oneshot::{channel, Sender},
    future, StreamExt,
};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::{task::spawn, time::timeout};

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

#[derive(Debug)]
pub enum RequestError {
    Timeout,
    SendError(SendError),
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
        if let Err(e) = self
            .peer
            .send_or_close(&request, |_| CloseReason::Other)
            .await
        {
            let mut state = self.state.lock();
            state.responses.remove(&request_identifier);
            return Err(RequestError::SendError(e));
        }

        // Now we only have to wait for the response.
        let response = timeout(self.timeout, receiver).await;

        // Lock state and remove channel on timeout.
        if let Err(_) = response {
            let mut state = self.state.lock();
            state.responses.remove(&request_identifier);
            return Err(RequestError::Timeout);
        }

        // Flatten response.
        response
            .ok()
            .map(|inner| inner.ok())
            .flatten()
            .ok_or(RequestError::ReceiveError)
    }
}
