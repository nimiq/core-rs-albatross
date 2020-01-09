use tungstenite::protocol::CloseFrame;

use network_messages::Message as NimiqMessage;

pub use self::client::nimiq_connect_async;
pub use self::error::Error;
pub use self::server::nimiq_accept_async;
pub use self::shared_stream::SharedNimiqMessageStream;
pub use self::stream::NimiqMessageStream;

pub mod websocket_connector;
mod reverse_proxy;
pub mod error;
pub mod public_state;
pub mod stream;
pub mod client;
pub mod server;
pub mod shared_stream;

/// This enum encapsulates the two types of messages we send over the channel:
/// - Nimiq messages
/// - Close frames
#[derive(Debug, Clone)]
pub enum Message {
    Message(NimiqMessage), // TODO: Box?
    Close(Option<CloseFrame<'static>>),
}

