use std::sync::Arc;

use beserial::{Serialize, Deserialize};
use hash::Blake2bHash;
use blockchain_base::AbstractBlockchain;
use blockchain_albatross::Blockchain as AlbatrossBlockchain;


pub trait SyncProtocol {
    type Block: Serialize + Deserialize;

    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash>;
    fn request_block(&self, block_hash: Blake2bHash) -> Self::Block;
}





struct FullSync<B: AbstractBlockchain<'static>> {
    blockchain: Arc<B>
}

impl<B: AbstractBlockchain<'static>> FullSync<B> {
    fn new(blockchain: Arc<B>) -> Self {
        Self {
            blockchain
        }
    }
}

impl<B: AbstractBlockchain<'static>> SyncProtocol for FullSync<B> {
    type Block = B::Block;

    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        self.blockchain.get_block_locators(max_count)
    }

    fn request_block(&self, block_hash: Blake2bHash) -> Self::Block {
        unimplemented!()
    }
}






struct MacroBlockSync<'env> {
    blockchain: Arc<AlbatrossBlockchain<'env>>
}

impl<'env> MacroBlockSync<'env> {
    fn new(blockchain: Arc<AlbatrossBlockchain<'env>>) -> Self {
        Self {
            blockchain
        }
    }
}

impl<'env> SyncProtocol for MacroBlockSync<'env> {
    type Block = <AlbatrossBlockchain<'env> as AbstractBlockchain<'env>>::Block;

    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        unimplemented!()
    }

    fn request_block(&self, block_hash: Blake2bHash) -> Self::Block {
        unimplemented!()
    }
}

