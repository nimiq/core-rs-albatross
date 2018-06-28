mod accounts_tree_node;
mod address_nibbles;
mod accounts_tree_store;
pub mod accounts_tree;

use beserial::{Serialize, Deserialize};

use self::accounts_tree_node::{AccountsTreeNode, NO_CHILDREN};
use self::address_nibbles::AddressNibbles;
use self::accounts_tree_store::AccountsTreeStore;

#[derive(Serialize,Deserialize)]
pub struct AccountsProof {}
