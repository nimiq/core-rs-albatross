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
use nimiq_utils::observer::NotifierStream;
use parking_lot::RwLock;
use tokio::sync::oneshot::{self, Receiver, Sender};

#[derive(Debug)]
pub enum ZKPComponentEvent {
    ElectionZKProof(
        u32,
        Blake2bHash,
        Result<Proof<MNT6_753>, NanoZKPError>,
        Vec<G2MNT6>,
    ),
}

pub struct ZKPComponentState {
    pub latest_pks: Vec<G2MNT6>,
    pub latest_header_hash: [u8; 32],
    pub latest_proof: Option<Proof<MNT6_753>>,
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
    pub zkp_state: Arc<RwLock<ZKPComponentState>>,
    genesis_state: Vec<u8>,
    receiver: Option<Receiver<(u32, Blake2bHash, Result<ZKPComponentState, NanoZKPError>)>>,
    blockchain_election_rx: NotifierStream<BlockchainEvent>,
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

        let blockchain_election_rx = blockchain.write().notifier.as_stream();

        Self {
            blockchain,
            zkp_state: Arc::new(RwLock::new(ZKPComponentState {
                latest_pks,
                latest_header_hash: genesis_block.header_hash,
                latest_proof: None,
            })),
            genesis_state,
            receiver: None,
            blockchain_election_rx,
        }
    }

    async fn generate_new_proof(
        block: MacroBlock,
        latest_pks: Vec<G2MNT6>,
        latest_header_hash: [u8; 32],
        previous_proof: Option<Proof<MNT6_753>>,
        genesis_state: Vec<u8>,
        sender: Sender<(u32, Blake2bHash, Result<ZKPComponentState, NanoZKPError>)>,
    ) {
        let final_pks: Vec<G2MNT6> = block
            .get_validators()
            .unwrap()
            .voting_keys()
            .into_iter()
            .map(|pub_key| pub_key.public_key)
            .collect(); //itodo

        let block = ZKPMacroBlock::try_from(&block).unwrap();
        log::info!("Starting to generate the proof");
        let proof = NanoZKP::prove(
            latest_pks.clone(),
            latest_header_hash,
            final_pks,
            block.clone(),
            previous_proof.map(|proof| (proof, genesis_state.clone())),
            true,
            true,
        );

        let new_block_blake_hash = block.header_hash.into();
        let result = match proof {
            Ok(proof) => sender.send((
                block.block_number,
                new_block_blake_hash,
                Ok(ZKPComponentState {
                    latest_pks,
                    latest_header_hash: block.header_hash,
                    latest_proof: Some(proof),
                }),
            )),
            Err(e) => sender.send((block.block_number, new_block_blake_hash, Err(e))),
        };

        if result.is_err() {
            log::error!(error = "receiver hung up", "error sending the zk proof",);
        }
    }
}

impl Stream for ZKPComponent {
    type Item = ZKPComponentEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        if let Some(ref mut receiver) = self.receiver {
            return match receiver.poll_unpin(cx) {
                Poll::Ready(Ok((block_number, block_hash, Ok(zkp_state)))) => {
                    self.receiver = None;
                    let mut zkp_state_write = self.zkp_state.write();
                    zkp_state_write.latest_pks = zkp_state.latest_pks;
                    zkp_state_write.latest_header_hash = zkp_state.latest_header_hash;
                    zkp_state_write.latest_proof = zkp_state.latest_proof;
                    drop(zkp_state_write);
                    let zkp_state_rg = self.zkp_state.read(); //ITODO change lock to downgradable ?

                    Poll::Ready(Some(ZKPComponentEvent::ElectionZKProof(
                        block_number,
                        block_hash,
                        zkp_state_rg
                            .latest_proof
                            .clone()
                            .ok_or(NanoZKPError::EmptyProof),
                        zkp_state_rg.latest_pks.clone(),
                    )))
                }
                Poll::Ready(Ok((block_number, block_hash, Err(e)))) => {
                    self.receiver = None;
                    Poll::Ready(Some(ZKPComponentEvent::ElectionZKProof(
                        block_number,
                        block_hash,
                        Err(e),
                        vec![],
                    )))
                }
                _ => Poll::Pending,
            };
        }

        while let Poll::Ready(event) = self.blockchain_election_rx.poll_next_unpin(cx) {
            match event {
                Some(BlockchainEvent::EpochFinalized(hash)) => {
                    let block = self.blockchain.read().get_block(&hash, true, None);
                    if let Some(Block::Macro(block)) = block {
                        let (sender, receiver) = oneshot::channel();
                        self.receiver = Some(receiver);
                        let zkp_state = self.zkp_state.read();
                        tokio::spawn(Self::generate_new_proof(
                            block,
                            zkp_state.latest_pks.clone(),
                            zkp_state.latest_header_hash,
                            zkp_state.latest_proof.clone(),
                            self.genesis_state.clone(),
                            sender,
                        ));
                    }
                }
                Some(_) => (),
                None => return Poll::Ready(None),
            }
        }
        Poll::Pending
    }
}
