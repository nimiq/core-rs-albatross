//! PoW migration test is a set of test that parses data from the PoW blockchain to
//! generate a PoS state.
//! These tests are gated under the `pow-migration-tests` since they require external
//! setup code.
#[cfg(feature = "pow-migration-tests")]
mod pow_migration_test {
    use nimiq_database::volatile::VolatileDatabase;
    use nimiq_genesis_builder::config::GenesisConfig;
    use nimiq_keys::Address;
    use nimiq_pow_migration::{migrate, BlockWindows, Error};
    use nimiq_primitives::networks::NetworkId;
    use nimiq_rpc::Client;
    use nimiq_test_log::test;
    use url::Url;

    fn setup_pow_client() -> Client {
        let url =
            std::env::var("POW_CLIENT_URL").expect("Missing `POW_CLIENT_URL` environment variable");
        Client::new(Url::parse(&url).expect("Could not parse provided URL"))
    }

    async fn test_pow_migration(
        block_windows: &BlockWindows,
        validator_address: &str,
        network_id: NetworkId,
        candidate_block: u32,
    ) -> Result<Option<GenesisConfig>, Error> {
        let env = VolatileDatabase::new(20).unwrap();
        let client = setup_pow_client();
        let address = Address::from_user_friendly_address(&validator_address)
            .expect("Could not parse provided validator address");
        migrate(
            &client,
            block_windows,
            candidate_block,
            env,
            &Some(address),
            network_id,
        )
        .await
    }

    #[test(tokio::test)]
    async fn window_1_testnet() {
        let validator_address = "NQ28 GSPY V07Q DJTK Y8TG DFYD KR5Q 9KBF HV5A";
        let block_windows = BlockWindows {
            registration_start: 2590000,
            registration_end: 2660000,
            pre_stake_start: 2660000,
            pre_stake_end: 2663100,
            election_candidate: 2664100,
            block_confirmations: 10,
            readiness_window: 10,
        };
        let network_id = NetworkId::TestAlbatross;
        let candidate_block = 2664100;
        let genesis = test_pow_migration(
            &block_windows,
            &validator_address,
            network_id,
            candidate_block,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(genesis.validators.len(), 2);
        assert_eq!(genesis.stakers.len(), 2);
        assert!(
            genesis
                .validators
                .iter()
                .find(
                    |validator| validator.validator_address.to_user_friendly_address()
                        == validator_address
                )
                .is_some(),
            "Could not find expected validator ('{}') in the genesis validator set",
            validator_address
        )
    }
}
