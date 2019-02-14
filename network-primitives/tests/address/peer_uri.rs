use std::str::FromStr;

use network_primitives::address::PeerUri;
use network_primitives::protocol::Protocol;



#[test]
fn test_parse_uri_dumb() {
    let uri = PeerUri::from_str("dumb://2b3f0f59334ef71ee7869b451139587f").unwrap();
    assert_eq!(uri.protocol(), Protocol::Dumb);
    assert_eq!(uri.hostname(), None);
    assert_eq!(uri.port(), None);
    assert_eq!(uri.peer_id(), Some(String::from("2b3f0f59334ef71ee7869b451139587f")).as_ref());
}

#[test]
fn test_parse_uri_rtc() {
    let uri = PeerUri::from_str("rtc://2b3f0f59334ef71ee7869b451139587f").unwrap();
    assert_eq!(uri.protocol(), Protocol::Rtc);
    assert_eq!(uri.hostname(), None);
    assert_eq!(uri.port(), None);
    assert_eq!(uri.peer_id(), Some(String::from("2b3f0f59334ef71ee7869b451139587f")).as_ref());
}

#[test]
fn test_parse_uri_ws_port_peerid() {
    let uri = PeerUri::from_str("ws://seed-20.nimiq.com:8443/2b3f0f59334ef71ee7869b451139587f").unwrap();
    assert_eq!(uri.protocol(), Protocol::Ws);
    assert_eq!(uri.hostname(), Some(String::from("seed-20.nimiq.com")).as_ref());
    assert_eq!(uri.port(), Some(8443));
    assert_eq!(uri.peer_id(), Some(String::from("2b3f0f59334ef71ee7869b451139587f")).as_ref());
}

#[test]
fn test_parse_uri_ws_peerid() {
    let uri = PeerUri::from_str("ws://seed-20.nimiq.com/2b3f0f59334ef71ee7869b451139587f").unwrap();
    assert_eq!(uri.protocol(), Protocol::Ws);
    assert_eq!(uri.hostname(), Some(String::from("seed-20.nimiq.com")).as_ref());
    assert_eq!(uri.port(), None);
    assert_eq!(uri.peer_id(), Some(String::from("2b3f0f59334ef71ee7869b451139587f")).as_ref());
}

#[test]
fn test_parse_uri_ws1() {
    let uri = PeerUri::from_str("ws://seed-20.nimiq.com/").unwrap();
    assert_eq!(uri.protocol(), Protocol::Ws);
    assert_eq!(uri.hostname(), Some(String::from("seed-20.nimiq.com")).as_ref());
    assert_eq!(uri.port(), None);
    assert_eq!(uri.peer_id(), None);
}

#[test]
fn test_parse_uri_ws2() {
    let uri = PeerUri::from_str("ws://seed-20.nimiq.com").unwrap();
    assert_eq!(uri.protocol(), Protocol::Ws);
    assert_eq!(uri.hostname(), Some(String::from("seed-20.nimiq.com")).as_ref());
    assert_eq!(uri.port(), None);
    assert_eq!(uri.peer_id(), None);
}

#[test]
fn test_parse_uri_ws() {
    let uri = PeerUri::from_str("wss://seed-20.nimiq.com/2b3f0f59334ef71ee7869b451139587f").unwrap();
    assert_eq!(uri.protocol(), Protocol::Wss);
    assert_eq!(uri.hostname(), Some(String::from("seed-20.nimiq.com")).as_ref());
    assert_eq!(uri.port(), None);
    assert_eq!(uri.peer_id(), Some(String::from("2b3f0f59334ef71ee7869b451139587f")).as_ref());
}

