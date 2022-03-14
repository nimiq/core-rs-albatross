use std::path::PathBuf;

use nimiq_lib::config::{
    config::{ClientConfigBuilder, DatabaseConfig, DatabaseConfigBuilder, FileStorageConfig},
    config_file::ConfigFile,
};
use nimiq_test_log::test;

#[test]
fn config_file_no_db_entry() {
    let config_file: ConfigFile = toml::from_str(r#""#).unwrap();
    let mut config_builder = ClientConfigBuilder::default();
    config_builder.config_file(&config_file).unwrap();
    let config = config_builder.build().unwrap();

    assert_eq!(config.database, DatabaseConfig::default());
}

#[test]
fn config_file_partial_db_entry() {
    // Only the DB entry
    let config_file: ConfigFile = toml::from_str(
        r#"
    [database]
    "#,
    )
    .unwrap();

    let mut config_builder = ClientConfigBuilder::default();
    config_builder.config_file(&config_file).unwrap();
    let config = config_builder.build().unwrap();

    assert_eq!(config.database, DatabaseConfig::default());

    // Set only the size
    let config_file: ConfigFile = toml::from_str(
        r#"
    [database]
    size = 4242
    "#,
    )
    .unwrap();

    let mut config_builder = ClientConfigBuilder::default();
    config_builder.config_file(&config_file).unwrap();
    let config = config_builder.build().unwrap();

    assert_eq!(
        config.database,
        DatabaseConfigBuilder::default()
            .size(4242usize)
            .build()
            .unwrap()
    );

    // Set only max_dbs
    let config_file: ConfigFile = toml::from_str(
        r#"
    [database]
    max_dbs = 10
    "#,
    )
    .unwrap();

    let mut config_builder = ClientConfigBuilder::default();
    config_builder.config_file(&config_file).unwrap();
    let config = config_builder.build().unwrap();

    assert_eq!(
        config.database,
        DatabaseConfigBuilder::default()
            .max_dbs(10u32)
            .build()
            .unwrap()
    );

    // Set only max_readers
    let config_file: ConfigFile = toml::from_str(
        r#"
    [database]
    max_readers = 42
    "#,
    )
    .unwrap();

    let mut config_builder = ClientConfigBuilder::default();
    config_builder.config_file(&config_file).unwrap();
    let config = config_builder.build().unwrap();

    assert_eq!(
        config.database,
        DatabaseConfigBuilder::default()
            .max_readers(42u32)
            .build()
            .unwrap()
    );

    // Set only the path
    let config_file: ConfigFile = toml::from_str(
        r#"
    [database]
    path = "/not/valid/path"
    "#,
    )
    .unwrap();

    let mut config_builder = ClientConfigBuilder::default();
    config_builder.config_file(&config_file).unwrap();
    let config = config_builder.build().unwrap();

    let mut db_config = FileStorageConfig::home();
    db_config.database_parent = PathBuf::from("/not/valid/path");

    assert_eq!(config.storage, db_config.into());
}
