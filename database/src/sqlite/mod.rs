use core::marker::PhantomData;
use std::path::Path;

use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::HashOutput;
use nimiq_transaction::historic_transaction::{
    HistoricTransaction, HistoricTransactionData, RawTransactionHash,
};
use rusqlite::{named_params, Connection, Statement};
use thiserror::Error;

pub type Result<T, E = Error> = core::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("SQLite error: {0}")]
    SQLite(#[from] rusqlite::Error),
    #[error("SQL type conversion error: {0}")]
    FromSql(#[from] rusqlite::types::FromSqlError),
    // #[error("encoding error: {0}")]
    // Encoding(#[from] serde_json::Error),
    #[error("transaction not found for hash")]
    HashNotFound,
}

pub struct SqliteDatabase {
    conn: Connection,
}

impl SqliteDatabase {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        Ok(SqliteDatabase { conn })
    }

    pub fn hist_txs_table(&self) -> Result<SqliteTable<'_>> {
        let name = "hist_txs";

        let create_sql = format!(
            r#"
            CREATE TABLE IF NOT EXISTS "{}" (
                "hash" BLOB NOT NULL PRIMARY KEY,
                "blocknumber" INTEGER NOT NULL,
                "sender" BLOB,
                "recipient" BLOB,
                "data" BLOB NOT NULL
            ) STRICT"#,
            name
        );
        self.conn.execute(&create_sql, [])?;

        // TODO: Investigate PRAGMA settings:
        // https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/
        // https://www.powersync.com/blog/sqlite-optimizations-for-ultra-high-performance
        // https://avi.im/blag/2021/fast-sqlite-inserts/#sqlite-optimisations
        self.conn.pragma_update(None, "journal_mode", "WAL")?;
        self.conn.pragma_update(None, "wal_autocheckpoint", 1_000_000)?; // Reduce this after sync is complete, otherwise read performance will be terrible
        self.conn.pragma_update(None, "synchronous", "NORMAL")?;
        // self.conn.pragma_update(None, "page_size", 1024 * 2 * 2 * 2)?; // 65536 is the maximum allowed
        self.conn.pragma_update(None, "cache_size", 1_000)?;
        self.conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;

        let get_by_hash_sql = format!(
            r#"
            SELECT "data"
            FROM "{}"
            WHERE
                "hash" = :hash
            "#,
            name
        );
        let get_by_hash_stmt = self.conn.prepare(&get_by_hash_sql)?;

        let insert_one_sql = format!(
            r#"
            INSERT INTO "{}" ("hash", "blocknumber", "sender", "recipient", "data")
            VALUES (:hash, :blocknumber, :sender, :recipient, :data)
            "#,
            name
        );
        let insert_one_stmt = self.conn.prepare(&insert_one_sql)?;

        let begin_transaction_stmt = self.conn.prepare("BEGIN EXCLUSIVE TRANSACTION")?;
        let commit_transaction_stmt = self.conn.prepare("COMMIT TRANSACTION")?;

        /*
         * For bootstrapping a database, another common trick is to drop all indexes, create the data, then
         * recreate the indexes. Some index types can be build faster when all of the data is known ahead of
         * time, versus a constant stream of new information that it has to inject into a balanced data structure.
         */
        let index_stmts = vec![
            self.conn.prepare(&format!(
                r#"
                CREATE INDEX IF NOT EXISTS "{}_blocknumber_idx"
                ON "{}"("blocknumber")
                "#,
                name, name
            ))?,
            self.conn.prepare(&format!(
                r#"
                CREATE INDEX IF NOT EXISTS "{}_sender_idx"
                ON "{}"("sender")
                "#,
                name, name
            ))?,
            self.conn.prepare(&format!(
                r#"
                CREATE INDEX IF NOT EXISTS "{}_recipient_idx"
                ON "{}"("recipient")
                "#,
                name, name
            ))?,
        ];

        let count_stmt = self.conn.prepare(&format!(
            r#"
            SELECT COUNT(*)
            FROM "{}"
            "#,
            name
        ))?;

        Ok(SqliteTable {
            get_by_hash_stmt,
            insert_one_stmt,
            begin_transaction_stmt,
            commit_transaction_stmt,
            index_stmts,
            count_stmt,
            buf: Vec::new(),
            marker: PhantomData,
        })
    }
}

pub struct SqliteTable<'store> {
    get_by_hash_stmt: Statement<'store>,
    insert_one_stmt: Statement<'store>,
    begin_transaction_stmt: Statement<'store>,
    commit_transaction_stmt: Statement<'store>,
    index_stmts: Vec<Statement<'store>>,
    count_stmt: Statement<'store>,
    buf: Vec<u8>,
    marker: PhantomData<fn() -> (RawTransactionHash, u32, HistoricTransaction)>,
}

impl<'store> SqliteTable<'store> {
    pub fn get_by_hash(&mut self, hash: &RawTransactionHash) -> Result<HistoricTransaction> {
        let mut rows = self
            .get_by_hash_stmt
            .query(named_params! {":hash": hash.as_bytes()})?;
        let maybe_row = rows.next()?;
        let row = maybe_row.ok_or(Error::HashNotFound)?;
        let bytes = row.get_ref("data")?.as_bytes()?;
        Ok(HistoricTransaction::copy_from_database(bytes).unwrap()) // TODO: Handle error
    }

    pub fn get_by_hash_opt(
        &mut self,
        hash: &RawTransactionHash,
    ) -> Result<Option<HistoricTransaction>> {
        let mut rows = self
            .get_by_hash_stmt
            .query(named_params! {":hash": hash.as_bytes()})?;
        let row = match rows.next()? {
            Some(r) => r,
            None => return Ok(None),
        };
        let bytes = row.get_ref("data")?.as_bytes()?;
        Ok(Some(
            HistoricTransaction::copy_from_database(bytes).unwrap(),
        )) // TODO: Handle error
    }

    pub fn insert_one(&mut self, hist_tx: &HistoricTransaction) -> Result<()> {
        // TODO: What is an MMRHash and what is the prefix of that? Use prefix for sharding?
        let hash = hist_tx.tx_hash();

        let (sender, recipient) = match hist_tx.data {
            HistoricTransactionData::Basic(ref tx) => {
                let raw_tx = tx.get_raw_transaction();
                (
                    Some(raw_tx.sender.as_bytes()),
                    Some(raw_tx.recipient.as_bytes()),
                )
            }
            HistoricTransactionData::Reward(ref tx) => (None, Some(tx.reward_address.as_bytes())),
            _ => {
                todo!();
            }
        };

        self.buf.clear();
        self.buf.resize(hist_tx.database_byte_size(), 0); // TODO: use rusqlite::blob feature instead
        hist_tx.copy_into_database(&mut self.buf);

        self.insert_one_stmt.execute(named_params! {
            ":hash": hash.as_database_bytes(),
            ":blocknumber": hist_tx.block_number,
            ":sender": sender,
            ":recipient": recipient,
            ":data": &self.buf[..],
        })?;

        Ok(())
    }

    pub fn insert(&mut self, hist_txs: &[HistoricTransaction]) -> Result<()> {
        for hist_tx in hist_txs {
            // TODO: Investigate SQLite transactions
            self.insert_one(hist_tx)?;
        }
        Ok(())
    }

    pub fn begin_transaction(&mut self) -> Result<()> {
        self.begin_transaction_stmt.execute([])?;
        Ok(())
    }

    pub fn commit(&mut self) -> Result<()> {
        self.commit_transaction_stmt.execute([])?;
        Ok(())
    }

    pub fn create_indexes(&mut self) -> Result<()> {
        for stmt in &mut self.index_stmts {
            stmt.execute([])?;
        }
        Ok(())
    }

    pub fn count(&mut self) -> Result<usize> {
        let mut rows = self.count_stmt.query([])?;
        let row = rows.next()?.unwrap();
        let count: i64 = row.get(0)?;
        Ok(count as usize)
    }
}
