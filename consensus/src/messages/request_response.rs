#[macro_export]
macro_rules! request_response {
    ($msg:ty) => {
        request_response!($msg, request_identifier);
    };
    ($msg:ty, $a:ident) => {
        impl nimiq_network_interface::message::RequestMessage for $msg {
            fn set_request_identifier(&mut self, request_identifier: u32) {
                self.$a = request_identifier;
            }
        }
        impl nimiq_network_interface::message::ResponseMessage for $msg {
            fn get_request_identifier(&self) -> u32 {
                self.$a
            }
        }
    };
}
