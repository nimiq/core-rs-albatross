use async_trait::async_trait;


#[cfg_attr(
    feature = "proxy",
    nimiq_jsonrpc_derive::proxy(name = "ValidatorProxy", rename_all = "camelCase")
)]
#[async_trait]
pub trait ValidatorInterface {
    type Error;
}
