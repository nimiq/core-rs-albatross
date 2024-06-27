use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    future::{BoxFuture, Future, FutureExt},
    stream::FuturesUnordered,
    StreamExt,
};
use nimiq_network_interface::request::{Request, RequestCommon};

use crate::ValidatorNetwork;

pub struct SingleResponseRequester<'a, TValidatorNetwork, TRequest, TContext, TOutput>
where
    TValidatorNetwork: ValidatorNetwork + 'static,
    TRequest: Request + Clone,
    TContext: Clone + Send + Unpin,
{
    network: Arc<TValidatorNetwork>,
    remaining_peers: Vec<u16>,
    request: TRequest,
    context: TContext,
    desired_pending_requests: usize,
    verification_fn: fn(<TRequest as RequestCommon>::Response, TContext) -> Option<TOutput>,

    pending_futures:
        FuturesUnordered<BoxFuture<'a, Result<<TRequest as RequestCommon>::Response, u16>>>,
}

impl<'a, TValidatorNetwork, TRequest, TContext, TOutput>
    SingleResponseRequester<'a, TValidatorNetwork, TRequest, TContext, TOutput>
where
    TValidatorNetwork: ValidatorNetwork + 'static,
    TRequest: Request + Clone,
    TContext: Clone + Send + Unpin,
{
    pub fn new(
        network: Arc<TValidatorNetwork>,
        remaining_peers: Vec<u16>,
        request: TRequest,
        context: TContext,
        desired_pending_requests: usize,
        verification_fn: fn(<TRequest as RequestCommon>::Response, TContext) -> Option<TOutput>,
    ) -> Self {
        let mut this = Self {
            network,
            remaining_peers,
            request,
            context,
            desired_pending_requests,
            verification_fn,
            pending_futures: FuturesUnordered::default(),
        };

        this.create_requests();

        this
    }

    fn create_requests(&mut self) {
        while !self.remaining_peers.is_empty()
            && self.pending_futures.len() < self.desired_pending_requests
        {
            self.create_request();
        }
    }

    fn create_request(&mut self) {
        let validator_id = self
            .remaining_peers
            .pop()
            .expect("create_request needs a non empty list of remaining peers.");

        let network = Arc::clone(&self.network);
        let request = self.request.clone();

        let future = async move {
            network
                .request(request, validator_id)
                .await
                .map_err(|_| validator_id)
        }
        .boxed();

        self.pending_futures.push(future);
    }
}

impl<'a, TValidatorNetwork, TRequest, TContext, TOutput> Future
    for SingleResponseRequester<'a, TValidatorNetwork, TRequest, TContext, TOutput>
where
    TValidatorNetwork: ValidatorNetwork + 'static,
    TRequest: Request + Clone,
    TContext: Clone + Send + Unpin,
{
    type Output = Option<TOutput>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            while let Poll::Ready(response) = self.pending_futures.poll_next_unpin(cx) {
                match response {
                    Some(Ok(response)) => {
                        if let Some(output) = (self.verification_fn)(response, self.context.clone())
                        {
                            return Poll::Ready(Some(output));
                        }
                        // If the verification_fn does not produce a result do nothing and try the next.
                    }
                    Some(Err(_validator_id)) => {
                        // Validator could not be reached.
                        // For now do nothing, later on this validator might be added back to the remaining peers
                        // to retry it a set amount of times.
                    }
                    None => {
                        // No more requests are pending.
                        // If there is also no more peers left to request from, this request has now failed as a whole.
                        if self.remaining_peers.is_empty() {
                            return Poll::Ready(None);
                        } else {
                            break;
                        }
                    }
                }
            }
            // In case no new requests need to be created, return Poll::Pending
            if self.remaining_peers.is_empty()
                || self.pending_futures.len() == self.desired_pending_requests
            {
                return Poll::Pending;
            }
            // Previous return was not triggered, thus requests must be created.
            self.create_requests();
        }
    }
}
