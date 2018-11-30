mod accounts_tree_node;
mod address_nibbles;
pub mod accounts_tree;

use beserial::{Serialize, Deserialize};

use self::accounts_tree_node::{AccountsTreeNode, NO_CHILDREN};
use self::address_nibbles::AddressNibbles;
pub use self::accounts_tree::AccountsTree;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountsProof {}
