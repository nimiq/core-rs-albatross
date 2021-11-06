extern crate nimiq_keys as keys;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_primitives as primitives;

use beserial::Deserialize;
use keys::Address;
use network_primitives::subscription_albatross::{
    Subscription, SubscriptionType, TransactionFilter,
};
use primitives::coin::Coin;
use std::collections::HashSet;
use std::convert::TryFrom;

// Nimiq.Subscription.fromAddresses([Nimiq.Address.fromUserFriendlyAddress("NQ43 P4RM 6KUV RP79 QMTR SE0K FJRQ K22P MD35"),Nimiq.Address.fromUserFriendlyAddress("NQ29 8MRT RYYN CNCE XRN5 DGYG HA2G CAVC MPDJ")]).serialize().toString()
const SUBSCRIPTION_ADDRESSES: &'static str =
    "0201000002b933534f9dcdce9c5779d38137cb3898857ab4654573bcfff66598ef66c56c3f08a85062bacaddb200";

// Nimiq.Subscription.fromMinFeePerByte(42).serialize().toString()
const SUBSCRIPTION_MINFEE: &'static str = "010101000000000000002a00";

const SUBSCRIPTION_NONE: &'static str = "000000";
const SUBSCRIPTION_HASHES: &'static str = "010001";
const SUBSCRIPTION_OBJECTS: &'static str = "020002";

#[test]
fn test_subscription_none() {
    let vec = hex::decode(SUBSCRIPTION_NONE).unwrap();
    let subscription: Subscription = Deserialize::deserialize(&mut &vec[..]).unwrap();
    assert_eq!(subscription.tx_announcements, SubscriptionType::None);
    assert_eq!(subscription.block_announcements, SubscriptionType::None);
    assert!(subscription.tx_filter.is_none());
}

#[test]
fn test_subscription_hashes() {
    let vec = hex::decode(SUBSCRIPTION_HASHES).unwrap();
    let subscription: Subscription = Deserialize::deserialize(&mut &vec[..]).unwrap();
    assert_eq!(subscription.tx_announcements, SubscriptionType::Hashes);
    assert_eq!(subscription.block_announcements, SubscriptionType::Hashes);
    assert!(subscription.tx_filter.is_none());
}

#[test]
fn test_subscription_objects() {
    let vec = hex::decode(SUBSCRIPTION_OBJECTS).unwrap();
    let subscription: Subscription = Deserialize::deserialize(&mut &vec[..]).unwrap();
    assert_eq!(subscription.tx_announcements, SubscriptionType::Objects);
    assert_eq!(subscription.block_announcements, SubscriptionType::Objects);
    assert!(subscription.tx_filter.is_none());
}

#[test]
fn test_subscription_minfee() {
    let vec = hex::decode(SUBSCRIPTION_MINFEE).unwrap();
    let subscription: Subscription = Deserialize::deserialize(&mut &vec[..]).unwrap();
    assert_eq!(subscription.tx_announcements, SubscriptionType::Hashes);
    assert_eq!(subscription.block_announcements, SubscriptionType::None);
    match subscription.tx_filter {
        Some(TransactionFilter::MinFee(fee)) => assert_eq!(Coin::try_from(42u64).unwrap(), fee),
        _ => assert!(false),
    };
}

#[test]
fn test_subscription_addresses() {
    let vec = hex::decode(SUBSCRIPTION_ADDRESSES).unwrap();
    let subscription: Subscription = Deserialize::deserialize(&mut &vec[..]).unwrap();
    assert_eq!(subscription.tx_announcements, SubscriptionType::Objects);
    assert_eq!(subscription.block_announcements, SubscriptionType::None);
    match subscription.tx_filter {
        Some(TransactionFilter::Addresses(addresses)) => {
            let mut expected = HashSet::with_capacity(2);
            expected.insert(
                Address::from_user_friendly_address(&String::from(
                    "NQ43 P4RM 6KUV RP79 QMTR SE0K FJRQ K22P MD35",
                ))
                .unwrap(),
            );
            expected.insert(
                Address::from_user_friendly_address(&String::from(
                    "NQ29 8MRT RYYN CNCE XRN5 DGYG HA2G CAVC MPDJ",
                ))
                .unwrap(),
            );
            assert_eq!(expected, addresses);
        }
        _ => assert!(false),
    };
}
