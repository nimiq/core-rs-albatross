use std::sync::Arc;
use std::time::SystemTime;

use json::{JsonValue, Null};

use beserial::{Deserialize, Serialize};
use block::{Block, BlockHeader};
use block_production::BlockProducer;
use hash::{Blake2bHash, Blake2bHasher, Hash};
use keys::Address;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain::PushResult;
use nimiq_mempool::Mempool;
use utils::merkle::MerklePath;
use utils::time::systemtime_to_timestamp;

use crate::handler::Method;
use crate::handlers::Module;
use crate::handlers::mempool::transaction_to_obj;

pub struct BlockProductionNimiqHandler {
    pub blockchain: Arc<Blockchain<'static>>,
    pub mempool: Arc<Mempool<'static, Blockchain<'static>>>,
}

impl BlockProductionNimiqHandler {
    pub fn new(blockchain: Arc<Blockchain<'static>>, mempool: Arc<Mempool<'static, Blockchain<'static>>>) -> Self {
        BlockProductionNimiqHandler {
            blockchain,
            mempool,
        }
    }

    /// Prepares a new block and returns a work object.
    /// {
    ///     data: string,
    ///     suffix: string,
    ///     target: number,
    ///     algorithm: string,
    /// }
    ///
    /// Parameters:
    /// - minerAddress (string)
    /// - extraData (string)
    pub(crate) fn get_work(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let block = self.produce_block(params)?;
        let block_bytes = block.serialize_to_vec();

        Ok(object!{
            "data" => hex::encode(&block_bytes[..BlockHeader::SIZE]),
            "suffix" => hex::encode(&block_bytes[BlockHeader::SIZE..]),
            "target" => u32::from(block.header.n_bits),
            "algorithm" => "nimiq-argon2",
        })
    }

    /// Prepares a block template, returning:
    /// {
    ///     header: {
    ///         version: number,
    ///         prevHash: string,
    ///         interlinkHash: string,
    ///         accountsHash: string,
    ///         nBits: number,
    ///         height: number,
    ///     },
    ///     interlink: string,
    ///     target: number,
    ///     body: {
    ///         hash: string,
    ///         minerAddr: string,
    ///         extraData: string,
    ///         transactions: Array<tx object>,
    ///         merkleHashes: Array<string>,
    ///         prunedAccounts: Array<string>,
    ///     },
    /// }
    ///
    /// Parameters:
    /// - minerAddress (string)
    /// - extraData (string)
    pub(crate) fn get_block_template(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let block = self.produce_block(params)?;
        let header = block.header;
        let json_header = object!{
            "version" => header.version,
            "prevHash" => header.prev_hash.to_hex(),
            "interlinkHash" => header.interlink_hash.to_hex(),
            "accountsHash" => header.accounts_hash.to_hex(),
            "nBits" => u32::from(header.n_bits),
            "height" => header.height,
        };
        let body = block.body.unwrap();
        let merkle_path = MerklePath::new::<Blake2bHasher, Blake2bHash>(&body.get_merkle_leaves::<Blake2bHash>(), &body.miner.hash::<Blake2bHash>());
        let json_body = object!{
            "hash" => header.body_hash.to_hex(),
            "minerAddr" => body.miner.to_hex(),
            "extraData" => hex::encode(body.extra_data),
            "transactions" => JsonValue::Array(body.transactions.iter().map(|tx| transaction_to_obj(tx, None, None)).collect()),
            "merkleHashes" => JsonValue::Array(merkle_path.hashes().iter().map(|hash| JsonValue::String(hash.to_hex())).skip(1).collect()),
            "prunedAccounts" => JsonValue::Array(body.receipts.receipts.iter().map(|acc| JsonValue::String(hex::encode(acc.serialize_to_vec()))).collect()),
        };

        Ok(object!{
            "header" => json_header,
            "interlink" => hex::encode(block.interlink.serialize_to_vec()),
            "target" => u32::from(header.n_bits),
            "body" => json_body,
        })
    }

    /// Submits a block to the blockchain.
    /// Parameters:
    /// - block (string)
    pub(crate) fn submit_block(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let block = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Block must be a string"})
            .and_then(|s| hex::decode(s)
                .map_err(|_| object!{"message" => "Block must be hex-encoded"}))
            .and_then(|b| Block::deserialize_from_vec(&b)
                .map_err(|_| object!{"message" => "Invalid block data"}))?;

        match self.blockchain.push(block) {
            Ok(PushResult::Forked) => Ok(object!{"message" => "Forked"}),
            Ok(_) => Ok(object!{"message" => "Ok"}),
            _ => Err(object!{"message" => "Block rejected"})
        }
    }

    fn produce_block(&self, params: &[JsonValue]) -> Result<Block, JsonValue> {
        let miner = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Miner address must be a string"})
            .and_then(|s| Address::from_any_str(s)
                .map_err(|_| object!{"message" => "Invalid miner address"}))?;

        let extra_data = params.get(1).unwrap_or(&Null).as_str()
            .ok_or_else(|| object!{"message" => "Extra data must be a string"})
            .and_then(|s| hex::decode(s)
                .map_err(|_| object!{"message" => "Extra data must be hex-encoded"}))?;

        let timestamp = (systemtime_to_timestamp(SystemTime::now()) / 1000) as u32;

        let producer = BlockProducer::new(self.blockchain.clone(), self.mempool.clone());
        Ok(producer.next_block(timestamp, miner, extra_data))
    }
}

impl Module for BlockProductionNimiqHandler {
    rpc_module_methods! {
        "getWork" => get_work,
        "getBlockTemplate" => get_block_template,
        "submitBlock" => submit_block
    }
}
