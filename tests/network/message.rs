use beserial::{Deserialize, Serialize};
use nimiq::network::message::*;

const VERSION_MESSAGE: &str = "4204204200000000e4ef6f9a59000000020000000000000000000000000000080808088f30a4d938d4130d1a3396dede1505c72b7f75ac9f9b80d1ad7e368b39f3b10500e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100bfafd3f1cc800309cb931081257b4e7edb0f9ea400187c550fd9141447ea086a5324dcf027dd4a30a932c441f365a25e86b173defa4b8e58948253471b81b72cf0000000000000000000000000000000000000000000000000000000000000000";
const INV_MESSAGE: &str = "42042042010000007b268c0610000300000002324dcf027dd4a30a932c441f365a25e86b173defa4b8e58948253471b81b72cf00000002b8b37c1d034e371c7a3b834f9476a746eb62259ff9558ab715b4bff79ebf58e100000001f823f66ba1026e7f711ea5aa4719837bb378fc615b50516b8dabdaff78e8168e";
const GET_DATA_MESSAGE: &str = "42042042020000007b990afcc2000300000002324dcf027dd4a30a932c441f365a25e86b173defa4b8e58948253471b81b72cf00000002b8b37c1d034e371c7a3b834f9476a746eb62259ff9558ab715b4bff79ebf58e100000001f823f66ba1026e7f711ea5aa4719837bb378fc615b50516b8dabdaff78e8168e";
const BLOCK_MESSAGE: &str = "4204204206000000ba2002774c0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000009d5b7130fdd19427406f1d477788ec1f866c650b2eff550afb3505b1435681bbbfacd81c643767f3bf1bf642c97b4efd33daeff2b0d341c613344ae706eedb381f010000000000010000000000018d5800011be440919634a6fe3ba5f8a7181fe4bb8212c13c0000000000";
const HEADER_MESSAGE: &str = "42042042070000009fc8a9c2ed0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000009d5b7130fdd19427406f1d477788ec1f866c650b2eff550afb3505b1435681bbbfacd81c643767f3bf1bf642c97b4efd33daeff2b0d341c613344ae706eedb381f010000000000010000000000018d58";
const TX_MESSAGE: &str = "42042042080000009803ad3340008f30a4d938d4130d1a3396dede1505c72b7f75ac9f9b80d1ad7e368b39f3b10591e9240f415223982edc345532630710e94a7f5200000000000022b8000000000000002a0000000004e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b00";
const GET_BLOCKS_MESSAGE: &str = "420420420500000012f392d9600000000402";
const MEMPOOL_MESSAGE: &str = "42042042090000000d994373bd";
const REJECT_MESSAGE: &str = "420420420a000000194422360c004104746573740003616263";
const ADDR_MESSAGE: &str = "4204204214000000f5650e831a00020000000000000000000000000000080808088f30a4d938d4130d1a3396dede1505c72b7f75ac9f9b80d1ad7e368b39f3b10500e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b0000000000000000000000000000080808088f30a4d938d4130d1a3396dede1505c72b7f75ac9f9b80d1ad7e368b39f3b10500e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";
const GET_ADDR_MESSAGE: &str = "420420421500000014c09a093a02000000040008";
const PING_MESSAGE: &str = "420420421600000011fde10bd200000002";
const PONG_MESSAGE: &str = "4204204217000000112077d25700000002";

const MESSAGES: [&str; 13] = [
    VERSION_MESSAGE,
    INV_MESSAGE,
    GET_DATA_MESSAGE,
    BLOCK_MESSAGE,
    HEADER_MESSAGE,
    TX_MESSAGE,
    GET_BLOCKS_MESSAGE,
    MEMPOOL_MESSAGE,
    REJECT_MESSAGE,
    ADDR_MESSAGE,
    GET_ADDR_MESSAGE,
    PING_MESSAGE,
    PONG_MESSAGE
];

#[test]
fn parse_version_message() {
    let vec = ::hex::decode(VERSION_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::Version(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_inv_message() {
    let hex_msg = "";
    let vec = ::hex::decode(INV_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::Inv(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_get_data_message() {
    let vec = ::hex::decode(GET_DATA_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::GetData(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_block_message() {
    let vec = ::hex::decode(BLOCK_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::Block(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_header_message() {
    let vec = ::hex::decode(HEADER_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::Header(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_tx_message() {
    let vec = ::hex::decode(TX_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::Tx(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_get_blocks_message() {
    let vec = ::hex::decode(GET_BLOCKS_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::GetBlocks(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_mempool_message() {
    let vec = ::hex::decode(MEMPOOL_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::Mempool => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_reject_message() {
    let vec = ::hex::decode(REJECT_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::Reject(_) => assert!(true), _ => assert!(false) };
}

//#[test]
//fn parse_subscribe_message() {
//    let hex_msg = "420420420b000000382a2e67c102000291e9240f415223982edc345532630710e94a7f5287298cc2f31fba73181ea2a9e6ef10dce21ed95e";
//    let vec = ::hex::decode(hex_msg).unwrap();
//    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
//    match message { Message::Subscribe(_) => assert!(true), _ => assert!(false) };
//}

#[test]
fn parse_addr_message() {
    let vec = ::hex::decode(ADDR_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::Addr(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_get_addr_message() {
    let vec = ::hex::decode(GET_ADDR_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::GetAddr(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_ping_message() {
    let vec = ::hex::decode(PING_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::Ping(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn parse_pong_message() {
    let vec = ::hex::decode(PONG_MESSAGE).unwrap();
    let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match message { Message::Pong(_) => assert!(true), _ => assert!(false) };
}

#[test]
fn reserialize_messages() {
    for message in MESSAGES.iter() {
        let vec = ::hex::decode(message).unwrap();
        let message: Message = Deserialize::deserialize(&mut &vec[..]).unwrap();
        assert!(message.serialize_to_vec() == vec);
    }
}
