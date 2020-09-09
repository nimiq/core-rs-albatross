use std::ops::Deref;

use beserial::{Deserialize, Serialize};

use super::{Message, RequestMessage, ResponseMessage};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestResponseMessage<T: Serialize + Deserialize> {
    pub msg: T,
    request_identifier: u32,
}

impl<T: Serialize + Deserialize> RequestResponseMessage<T> {
    pub fn new(msg: T) -> Self {
        RequestResponseMessage {
            msg,
            request_identifier: 0,
        }
    }

    pub fn with_identifier(msg: T, request_identifier: u32) -> Self {
        RequestResponseMessage {
            msg,
            request_identifier,
        }
    }
}

impl<T: Serialize + Deserialize> Deref for RequestResponseMessage<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.msg
    }
}

impl<T: Serialize + Deserialize> RequestMessage for RequestResponseMessage<T>
where
    RequestResponseMessage<T>: Message,
{
    fn set_request_identifier(&mut self, request_identifier: u32) {
        self.request_identifier = request_identifier;
    }
}

impl<T: Serialize + Deserialize> ResponseMessage for RequestResponseMessage<T>
where
    RequestResponseMessage<T>: Message,
{
    fn get_request_identifier(&self) -> u32 {
        self.request_identifier
    }
}
