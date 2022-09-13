use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{FutureExt, Stream, StreamExt};

use ark_groth16::Proof;
use ark_mnt6_753::{G2Projective as G2MNT6, MNT6_753};
use nimiq_block::{Block, MacroBlock};
use nimiq_hash::Blake2bHash;
use nimiq_nano_primitives::{state_commitment, MacroBlock as ZKPMacroBlock};

use nimiq_blockchain::{AbstractBlockchain, Blockchain, BlockchainEvent};
use nimiq_nano_zkp::{NanoZKP, NanoZKPError};
use parking_lot::RwLock;
use tokio::sync::oneshot::{self, Receiver, Sender};

#[derive(Debug)]
pub enum ZKPComponentEvent {
    ElectionZKProof(Blake2bHash, Result<Proof<MNT6_753>, NanoZKPError>),
}

pub struct ZKPComponentState {
    latest_pks: Vec<G2MNT6>,
    latest_header_hash: [u8; 32],
    previous_proof: Option<Proof<MNT6_753>>,
}

/// Election blocks ZKP Component. Has:
///
/// - The previous election block public keys
/// - Previous election block header hash
/// - The pk tree root at genesis or none if genesis block
///
/// The proofs are returned by polling the components.
pub struct ZKPComponent {
    blockchain: Arc<RwLock<Blockchain>>,
    zkp_state: ZKPComponentState,
    genesis_state: Vec<u8>,
    receiver: Option<Receiver<(Blake2bHash, Result<ZKPComponentState, NanoZKPError>)>>,
}

impl ZKPComponent {
    pub fn new(blockchain: Arc<RwLock<Blockchain>>, genesis_block: MacroBlock) -> Self {
        let latest_pks: Vec<_> = genesis_block
            .get_validators()
            .unwrap()
            .voting_keys()
            .into_iter()
            .map(|pub_key| pub_key.public_key)
            .collect(); //itodo

        let genesis_block = ZKPMacroBlock::try_from(&genesis_block).unwrap();
        let genesis_state = state_commitment(
            genesis_block.block_number,
            genesis_block.header_hash,
            latest_pks.clone(),
        );

        Self {
            blockchain,
            zkp_state: ZKPComponentState {
                latest_pks,
                latest_header_hash: genesis_block.header_hash,
                previous_proof: None,
            },
            genesis_state,
            receiver: None,
        }
    }

    async fn generate_new_proof(
        block: MacroBlock,
        latest_pks: Vec<G2MNT6>,
        latest_header_hash: [u8; 32],
        previous_proof: Option<Proof<MNT6_753>>,
        genesis_state: Vec<u8>,
        sender: Sender<(Blake2bHash, Result<ZKPComponentState, NanoZKPError>)>,
    ) {
        let final_pks: Vec<G2MNT6> = block
            .get_validators()
            .unwrap()
            .voting_keys()
            .into_iter()
            .map(|pub_key| pub_key.public_key)
            .collect(); //itodo

        let new_block_hash = block.hash();
        let block = ZKPMacroBlock::try_from(&block).unwrap();

        let proof = NanoZKP::prove(
            latest_pks.clone(),
            latest_header_hash.clone(),
            final_pks.clone(),
            block.clone(),
            previous_proof
                .clone()
                .map(|proof| (proof, genesis_state.clone())),
            true,
            true,
        );

        let result = match proof {
            Ok(proof) => sender.send((
                new_block_hash,
                Ok(ZKPComponentState {
                    latest_pks,
                    latest_header_hash,
                    previous_proof: Some(proof),
                }),
            )),
            Err(e) => sender.send((new_block_hash, Err(e))),
        };

        if result.is_err() {
            log::error!(error = "receiver hung up", "error sending the zk proof",);
        }
    }
}

impl Stream for ZKPComponent {
    type Item = ZKPComponentEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let mut blockchain_election_rx = self.blockchain.write().notifier.as_stream();

        if let Some(ref mut receiver) = self.receiver {
            return match receiver.poll_unpin(cx) {
                Poll::Ready(Ok((block_hash, Ok(zkp_state)))) => {
                    self.receiver = None;
                    self.zkp_state = zkp_state;
                    Poll::Ready(Some(ZKPComponentEvent::ElectionZKProof(
                        block_hash,
                        self.zkp_state
                            .previous_proof
                            .clone()
                            .ok_or(NanoZKPError::EmptyProof),
                    )))
                }
                Poll::Ready(Ok((block_hash, Err(e)))) => {
                    self.receiver = None;
                    Poll::Ready(Some(ZKPComponentEvent::ElectionZKProof(block_hash, Err(e))))
                }
                _ => Poll::Pending,
            };
        }

        //concurrently launch at every event
        match blockchain_election_rx.poll_next_unpin(cx) {
            Poll::Ready(Some(BlockchainEvent::EpochFinalized(hash))) => {
                let block = self.blockchain.read().get_block(&hash, true, None);
                if let Some(Block::Macro(block)) = block {
                    let (sender, receiver) = oneshot::channel();
                    self.receiver = Some(receiver);

                    tokio::spawn(Self::generate_new_proof(
                        block,
                        self.zkp_state.latest_pks.clone(),
                        self.zkp_state.latest_header_hash,
                        self.zkp_state.previous_proof.clone(),
                        self.genesis_state.clone(),
                        sender,
                    ));
                }
            }
            Poll::Ready(None) => return Poll::Ready(None),
            _ => {}
        }
        Poll::Pending
    }
}
