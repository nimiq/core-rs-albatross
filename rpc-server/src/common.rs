use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use hex;
use json::{Array, JsonValue, Null};
use json::object::Object;
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use blockchain_base::AbstractBlockchain;
use consensus::{Consensus, ConsensusProtocol};
use hash::{Blake2bHash, Hash};
use mempool::ReturnCode;
use network::address::peer_address_state::{PeerAddressInfo, PeerAddressState};
use network::connection::close_type::CloseType;
use network::connection::connection_info::ConnectionInfo;
use network::connection::connection_pool::ConnectionId;
use network::peer_scorer::Score;
use network_primitives::address::{PeerId, PeerUri};
use transaction::Transaction;

use crate::rpc_not_implemented;
use crate::{JsonRpcConfig, JsonRpcServerState};

pub struct RpcHandler<P: ConsensusProtocol + 'static> {
    pub state: Arc<RwLock<JsonRpcServerState>>,
    pub consensus: Arc<Consensus<P>>,
    pub starting_block: u32,
    pub config: Arc<JsonRpcConfig>,
}

impl<P: ConsensusProtocol + 'static> RpcHandler<P> {

    // Network

    pub(crate) fn peer_count(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.consensus.network.peer_count().into())
    }

    pub(crate) fn consensus(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.state.read().consensus_state.into())
    }

    pub(crate) fn syncing(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(if self.state.read().consensus_state == "established" {
            false.into()
        }
        else {
            let current_block = self.consensus.blockchain.head_height();
            object! {
                "starting_block" => self.starting_block,
                "current_block" => current_block,
                "highest_block" => current_block // TODO
            }
        })
    }

    pub(crate) fn peer_list(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        let mut scores: HashMap<ConnectionId, Score> = HashMap::new();
        for (id, score) in self.consensus.network.scorer().connection_scores() {
            scores.insert(*id, *score);
        }

        Ok(self.consensus.network.addresses.state().address_info_iter()
            .map(|info| {
                let conn_id = self.consensus.network.connections.state()
                    .get_connection_id_by_peer_address(&info.peer_address);
                self.peer_address_info_to_obj(info, None,
                                              conn_id.and_then(|id| scores.get(&id)).map(|s| *s))
            })
            .collect::<Array>().into())
    }

    pub(crate) fn peer_state(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let peer_uri = params.get(0).unwrap_or(&Null).as_str()
            .ok_or_else(|| object!{"message" => "Invalid peer URI"})
            .and_then(|uri| PeerUri::from_str(uri)
                .map_err(|e| object!{"message" => e.to_string()}))?;

        let peer_id = peer_uri.peer_id()
            .ok_or_else(|| object!{"message" => "URI must contain peer ID"})
            .and_then(|s| PeerId::from_str(s)
                .map_err(|e| object!{"message" => e.to_string()}))?;

        let mut address_book = self.consensus.network.addresses.state_mut();
        let peer_address = address_book.get_by_peer_id(&peer_id)
            .ok_or_else(|| object!{"message" => "Unknown peer"})?;
        let mut peer_address_info = address_book.get_info_mut(&peer_address)
            .ok_or_else(|| object!{"message" => "Unknown peer"})?;


        let connection_pool = self.consensus.network.connections.state();
        let connection_info = connection_pool.get_connection_by_peer_address(&peer_address_info.peer_address);
        let peer_channel = connection_info.and_then(|c| c.peer_channel());

        let set = params.get(1).unwrap_or(&Null);
        if !set.is_null() {
            let set = set.as_str().ok_or_else(|| object!{"message" => "Invalid value for 'set'"})?;
            match set {
                "disconnect" => {
                    peer_channel.map(|p| p.close(CloseType::ManualPeerDisconnect));
                },
                "fail" => {
                    peer_channel.map(|p| p.close(CloseType::ManualPeerFail));
                },
                "ban" => {
                    peer_channel.map(|p| p.close(CloseType::ManualPeerBan));
                },
                "unban" => {
                    if peer_address_info.state == PeerAddressState::Banned {
                        peer_address_info.state = PeerAddressState::Tried;
                    }
                },
                "connect" => {
                    drop(address_book);
                    drop(connection_pool);
                    self.consensus.network.connections.connect_outbound(peer_address);
                }
                _ => return Err(object!{"message" => "Unknown 'set' command."})
            }
            Ok(Null)
        }
        else {
            Ok(self.peer_address_info_to_obj(peer_address_info, connection_info, None))
        }
    }

    // Transaction

    pub(crate) fn mempool_content(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let include_transactions = params.get(0).and_then(JsonValue::as_bool)
            .unwrap_or(false);

        Ok(JsonValue::Array(self.consensus.mempool.get_transactions(usize::max_value(), 0f64)
            .iter()
            .map(|tx| if include_transactions {
                self.transaction_to_obj(tx, None, None)
            } else {
                tx.hash::<Blake2bHash>().to_hex().into()
            })
            .collect::<Array>()))
    }

    pub(crate) fn mempool(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        // Transactions sorted by fee/byte, ascending
        let transactions = self.consensus.mempool.get_transactions(usize::max_value(), 0f64);
        let bucket_values: [u64; 14] = [0, 1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000];
        let mut bucket_counts: [u32; 14] = [0; 14];
        let mut i = 0;

        for transaction in &transactions {
            while (transaction.fee_per_byte() as u64) < bucket_values[i] { i += 1; }
            bucket_counts[i] += 1;
        }

        let mut transactions_per_bucket = Object::new();
        let mut buckets = Array::new();
        transactions_per_bucket.insert("total", transactions.len().into());
        for (&value, &count) in bucket_values.iter().zip(&bucket_counts) {
            if count > 0 {
                transactions_per_bucket.insert(value.to_string().as_str(), JsonValue::from(count));
                buckets.push(value.into());
            }
        }
        transactions_per_bucket.insert("buckets", JsonValue::Array(buckets));

        Ok(JsonValue::Object(transactions_per_bucket))
    }

    pub(crate) fn send_raw_transaction(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let raw = hex::decode(params.get(0)
            .unwrap_or(&Null)
            .as_str()
            .ok_or_else(|| object!{"message" => "Raw transaction must be a string"} )?)
            .map_err(|_| object!{"message" => "Raw transaction must be a hex string"} )?;
        let transaction: Transaction = Deserialize::deserialize_from_vec(&raw)
            .map_err(|_| object!{"message" => "Transaction can't be deserialized"} )?;
        self.push_transaction(transaction)
    }

    pub(crate) fn create_raw_transaction(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let raw = Serialize::serialize_to_vec(&self.obj_to_transaction(params.get(0).unwrap_or(&Null))?);
        Ok(hex::encode(raw).into())
    }

    pub(crate) fn send_transaction(&self, params: Array) -> Result<JsonValue, JsonValue> {
        self.push_transaction(self.obj_to_transaction(params.get(0).unwrap_or(&Null))?)
    }

    // Blockchain

    pub(crate) fn block_number(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.consensus.blockchain.head_height().into())
    }

    // Helper functions

    fn push_transaction(&self, transaction: Transaction) -> Result<JsonValue, JsonValue> {
        match self.consensus.mempool.push_transaction(transaction) {
            ReturnCode::Accepted | ReturnCode::Known => Ok(object!{"message" => "Ok"}),
            code => Err(object!{"message" => format!("Rejected: {:?}", code)})
        }
    }

    // TODO pub(crate) fn transaction_to_obj<B: BaseBlockHeader>(&self, transaction: &Transaction, block: Option<&B>, i: Option<usize>) -> JsonValue {
    pub(crate) fn transaction_to_obj(&self, transaction: &Transaction, block_number: Option<u32>, block_idx: Option<usize>) -> JsonValue {
        //let header = block.as_ref().map(|b| &b.header);
        object!{
            "hash" => transaction.hash::<Blake2bHash>().to_hex(),
            // TODO "blockHash" => header.map(|h| h.hash().to_hex().into()).unwrap_or(Null),
            // "blockNumber" => header.map(|h| h.height().into()).unwrap_or(Null),
            "blockNumber" => block_number.map(|n| n.into()).unwrap_or(Null),
            // TODO "timestamp" => header.map(|h| h.timestamp.into()).unwrap_or(Null),
            // "confirmations" => header.map(|b| (self.consensus.blockchain.head_height() - b.height()).into()).unwrap_or(Null),
            "confirmations" => block_number.map(|n| (self.consensus.blockchain.head_height() - n).into()).unwrap_or(Null),
            "transactionIndex" => block_idx.map(|i| i.into()).unwrap_or(Null),
            "from" => transaction.sender.to_hex(),
            "fromAddress" => transaction.sender.to_user_friendly_address(),
            "to" => transaction.recipient.to_hex(),
            "toAddress" => transaction.recipient.to_user_friendly_address(),
            "value" => u64::from(transaction.value),
            "fee" => u64::from(transaction.fee),
            "data" => hex::encode(&transaction.data),
            "flags" => transaction.flags.bits(),
            "validityStartHeight" => transaction.validity_start_height
        }
    }

    pub(crate) fn obj_to_transaction(&self, _obj: &JsonValue) -> Result<Transaction, JsonValue> {
        /*
        let from = Address::from_any_str(obj["from"].as_str()
            .ok_or_else(|| object!{"message" => "Sender address must be a string"})?)
            .map_err(|_|  object!{"message" => "Sender address invalid"})?;

        let from_type = match &obj["fromType"] {
            &JsonValue::Null => Some(AccountType::Basic),
            n @ JsonValue::Number(_) => n.as_u8().and_then(|n| AccountType::from_int(n)),
            _ => None
        }.ok_or_else(|| object!{"message" => "Invalid sender account type"})?;

        let to = Address::from_any_str(obj["to"].as_str()
            .ok_or_else(|| object!{"message" => "Recipient address must be a string"})?)
            .map_err(|_|  object!{"message" => "Recipient address invalid"})?;

        let to_type = match &obj["toType"] {
            &JsonValue::Null => Some(AccountType::Basic),
            n @ JsonValue::Number(_) => n.as_u8().and_then(|n| AccountType::from_int(n)),
            _ => None
        }.ok_or_else(|| object!{"message" => "Invalid recipient account type"})?;

        let value = Coin::from(obj["value"].as_u64()
            .ok_or_else(|| object!{"message" => "Invalid transaction value"})?);

        let fee = Coin::from(obj["value"].as_u64()
            .ok_or_else(|| object!{"message" => "Invalid transaction fee"})?);

        let flags = obj["flags"].as_u8()
            .map_or_else(|| Some(TransactionFlags::empty()), TransactionFlags::from_bits)
            .ok_or_else(|| object!{"message" => "Invalid transaction flags"})?;

        let data = obj["data"].as_str()
            .map(|d| hex::decode(d))
            .transpose().map_err(|_| object!{"message" => "Invalid transaction data"})?
            .unwrap_or(vec![]);
        */

        rpc_not_implemented()
    }

    pub(crate) fn peer_address_info_to_obj(&self, peer_address_info: &PeerAddressInfo, connection_info: Option<&ConnectionInfo<P::Blockchain>>, score: Option<Score>) -> JsonValue {
        let state = self.consensus.network.connections.state();
        let connection_info = connection_info.or_else(|| {
            state.get_connection_by_peer_address(&peer_address_info.peer_address)
        });
        let peer = connection_info.and_then(|conn| conn.peer());

        object!{
            "id" => peer_address_info.peer_address.peer_id().to_hex(),
            "address" => peer_address_info.peer_address.as_uri().to_string(),
            "failedAttempts" => peer_address_info.failed_attempts,
            "addressState" => peer_address_info.state as u8,
            "connectionState" => connection_info.map(|conn| (conn.state() as u8).into()).unwrap_or(Null),
            "version" => peer.map(|peer| peer.version.into()).unwrap_or(Null),
            "timeOffset" => peer.map(|peer| peer.time_offset.into()).unwrap_or(Null),
            "headHash" => peer.map(|peer| peer.head_hash.to_hex().into()).unwrap_or(Null),
            "score" => score.map(|s| s.into()).unwrap_or(Null),
            "latency" => connection_info.map(|conn| conn.statistics().latency_median().into()).unwrap_or(Null),
            "rx" => Null, // TODO: Not in NetworkConnection
            "tx" => Null,
        }
    }

}
