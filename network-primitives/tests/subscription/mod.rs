extern crate nimiq_keys as keys;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_primitives as primitives;

use beserial::Deserialize;
use keys::Address;
use network_primitives::subscription::{Subscription, SubscriptionType};
use primitives::coin::Coin;
use std::collections::HashSet;
use std::convert::TryFrom;

// Nimiq.Subscription.fromAddresses([Nimiq.Address.fromUserFriendlyAddress("NQ43 P4RM 6KUV RP79 QMTR SE0K FJRQ K22P MD35"),Nimiq.Address.fromUserFriendlyAddress("NQ29 8MRT RYYN CNCE XRN5 DGYG HA2G CAVC MPDJ")]).serialize().toString()
const SUBSCRIPTION_ADDRESSES: &'static str =
    "020002b933534f9dcdce9c5779d38137cb3898857ab4654573bcfff66598ef66c56c3f08a85062bacaddb2";

// Nimiq.Subscription.fromMinFeePerByte(42).serialize().toString()
const SUBSCRIPTION_MINFEE: &'static str = "03000000000000002a";

// Subscription::None
const SUBSCRIPTION_NONE: &'static str = "00";

// Subscription::Any
const SUBSCRIPTION_ANY: &'static str = "01";

#[test]
fn test_subscription_none() {
    let vec = hex::decode(SUBSCRIPTION_NONE).unwrap();
    let subscription: Subscription = Deserialize::deserialize(&mut &vec[..]).unwrap();
    assert_eq!(SubscriptionType::None, subscription.subscription_type());
}

#[test]
fn test_subscription_any() {
    let vec = hex::decode(SUBSCRIPTION_ANY).unwrap();
    let subscription: Subscription = Deserialize::deserialize(&mut &vec[..]).unwrap();
    assert_eq!(SubscriptionType::Any, subscription.subscription_type());
}

#[test]
fn test_subscription_minfee() {
    let vec = hex::decode(SUBSCRIPTION_MINFEE).unwrap();
    let subscription: Subscription = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match subscription {
        Subscription::MinFee(fee) => assert_eq!(Coin::try_from(42u64).unwrap(), fee),
        _ => assert!(false),
    };
}

#[test]
fn test_subscription_addresses() {
    let vec = hex::decode(SUBSCRIPTION_ADDRESSES).unwrap();
    let subscription: Subscription = Deserialize::deserialize(&mut &vec[..]).unwrap();
    match subscription {
        Subscription::Addresses(addresses) => {
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
