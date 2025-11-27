# Keccak256 Merkle Proof Examples

This document provides practical examples of using Keccak256 Merkle proofs with various Ethereum-compatible tools and libraries.

## Table of Contents

- [JavaScript/TypeScript Examples](#javascripttypescript-examples)
- [Python Examples](#python-examples)
- [Rust Examples](#rust-examples)
- [Solidity Smart Contract Examples](#solidity-smart-contract-examples)
- [Command Line Examples](#command-line-examples)

## JavaScript/TypeScript Examples

### Using ethers.js v5

```javascript
const { ethers } = require('ethers');

/**
 * Serialize a Nimiq transaction for hashing
 * This is a simplified example - actual serialization depends on transaction type
 */
function serializeTransaction(tx) {
  // Implement according to Nimiq transaction serialization format
  // This is transaction-type specific
  const data = ethers.utils.concat([
    ethers.utils.arrayify(tx.sender),
    ethers.utils.arrayify(tx.recipient),
    ethers.utils.hexZeroPad(ethers.utils.hexlify(tx.value), 8),
    // ... other fields
  ]);
  return data;
}

/**
 * Verify a Keccak256 Merkle proof
 */
function verifyKeccak256Proof(proof, root) {
  // Start with the transaction hash
  let currentHash = ethers.utils.keccak256(
    serializeTransaction(proof.transaction)
  );
  
  console.log('Starting with transaction hash:', currentHash);
  
  // Process each sibling hash in the proof
  for (let i = 0; i < proof.hashes.length; i++) {
    const siblingHash = proof.hashes[i];
    
    // Sort hashes lexicographically
    const hashes = [currentHash, siblingHash].sort();
    
    console.log(`Level ${i}:`);
    console.log('  Current:', currentHash);
    console.log('  Sibling:', siblingHash);
    console.log('  Sorted:', hashes);
    
    // Concatenate and hash
    currentHash = ethers.utils.keccak256(
      ethers.utils.concat([hashes[0], hashes[1]])
    );
    
    console.log('  Result:', currentHash);
  }
  
  console.log('Final computed root:', currentHash);
  console.log('Expected root:', root);
  
  return currentHash === root;
}

/**
 * Fetch and verify a transaction proof from Nimiq RPC
 */
async function fetchAndVerifyProof(epochNumber, transactionIndex) {
  const rpcUrl = 'http://localhost:8648';
  
  // Fetch the proof
  const proofResponse = await fetch(rpcUrl, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      jsonrpc: '2.0',
      method: 'getKeccak256TransactionProof',
      params: [epochNumber, transactionIndex],
      id: 1
    })
  });
  
  const proofData = await proofResponse.json();
  const proof = proofData.result;
  
  // Fetch the root
  const rootResponse = await fetch(rpcUrl, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      jsonrpc: '2.0',
      method: 'getKeccak256HistoryRoot',
      params: [epochNumber],
      id: 2
    })
  });
  
  const rootData = await rootResponse.json();
  const root = rootData.result;
  
  // Verify the proof
  const isValid = verifyKeccak256Proof(proof, root);
  
  console.log('Proof verification:', isValid ? 'VALID' : 'INVALID');
  
  return isValid;
}

// Usage
fetchAndVerifyProof(42, 5)
  .then(valid => console.log('Transaction proof is', valid ? 'valid' : 'invalid'))
  .catch(err => console.error('Error:', err));
```

### Using ethers.js v6

```typescript
import { ethers } from 'ethers';

interface MerkleProof {
  hashes: string[];
  transaction: any;
}

async function verifyProofV6(
  proof: MerkleProof,
  root: string
): Promise<boolean> {
  // Start with the transaction hash
  let currentHash = ethers.keccak256(
    serializeTransaction(proof.transaction)
  );
  
  // Process each sibling hash
  for (const siblingHash of proof.hashes) {
    // Sort hashes lexicographically
    const hashes = [currentHash, siblingHash].sort();
    
    // Concatenate and hash
    currentHash = ethers.keccak256(
      ethers.concat([hashes[0], hashes[1]])
    );
  }
  
  return currentHash === root;
}
```

### Using web3.js

```javascript
const Web3 = require('web3');
const web3 = new Web3();

/**
 * Verify a Keccak256 Merkle proof using web3.js
 */
function verifyProofWeb3(proof, root) {
  // Start with the transaction hash
  let currentHash = web3.utils.keccak256(
    serializeTransaction(proof.transaction)
  );
  
  // Process each sibling hash
  for (const siblingHash of proof.hashes) {
    // Convert to bytes
    const current = web3.utils.hexToBytes(currentHash);
    const sibling = web3.utils.hexToBytes(siblingHash);
    
    // Sort lexicographically
    const sorted = [current, sibling].sort((a, b) => {
      for (let i = 0; i < a.length; i++) {
        if (a[i] !== b[i]) return a[i] - b[i];
      }
      return 0;
    });
    
    // Concatenate and hash
    const combined = new Uint8Array([...sorted[0], ...sorted[1]]);
    currentHash = web3.utils.keccak256(combined);
  }
  
  return currentHash === root;
}
```

## Python Examples

### Using web3.py

```python
from web3 import Web3
from typing import List, Dict, Any

def serialize_transaction(tx: Dict[str, Any]) -> bytes:
    """
    Serialize a Nimiq transaction for hashing.
    This is a simplified example - actual serialization depends on transaction type.
    """
    # Implement according to Nimiq transaction serialization format
    # This is transaction-type specific
    pass

def verify_keccak256_proof(proof: Dict[str, Any], root: str) -> bool:
    """
    Verify a Keccak256 Merkle proof.
    
    Args:
        proof: Dictionary containing 'hashes' and 'transaction'
        root: Expected root hash (hex string with '0x' prefix)
    
    Returns:
        True if proof is valid, False otherwise
    """
    # Start with the transaction hash
    current_hash = Web3.keccak(serialize_transaction(proof['transaction']))
    
    print(f"Starting with transaction hash: {current_hash.hex()}")
    
    # Process each sibling hash in the proof
    for i, sibling_hash in enumerate(proof['hashes']):
        # Convert sibling hash from hex string to bytes
        sibling_bytes = bytes.fromhex(sibling_hash[2:])  # Remove '0x' prefix
        
        # Sort hashes lexicographically
        hashes = sorted([current_hash, sibling_bytes])
        
        print(f"Level {i}:")
        print(f"  Current: 0x{current_hash.hex()}")
        print(f"  Sibling: {sibling_hash}")
        
        # Concatenate and hash
        current_hash = Web3.keccak(hashes[0] + hashes[1])
        
        print(f"  Result: 0x{current_hash.hex()}")
    
    computed_root = '0x' + current_hash.hex()
    print(f"Final computed root: {computed_root}")
    print(f"Expected root: {root}")
    
    return computed_root == root

def fetch_and_verify_proof(
    rpc_url: str,
    epoch_number: int,
    transaction_index: int
) -> bool:
    """
    Fetch and verify a transaction proof from Nimiq RPC.
    """
    import requests
    
    # Fetch the proof
    proof_response = requests.post(rpc_url, json={
        'jsonrpc': '2.0',
        'method': 'getKeccak256TransactionProof',
        'params': [epoch_number, transaction_index],
        'id': 1
    })
    proof = proof_response.json()['result']
    
    # Fetch the root
    root_response = requests.post(rpc_url, json={
        'jsonrpc': '2.0',
        'method': 'getKeccak256HistoryRoot',
        'params': [epoch_number],
        'id': 2
    })
    root = root_response.json()['result']
    
    # Verify the proof
    is_valid = verify_keccak256_proof(proof, root)
    
    print(f"Proof verification: {'VALID' if is_valid else 'INVALID'}")
    
    return is_valid

# Usage
if __name__ == '__main__':
    rpc_url = 'http://localhost:8648'
    is_valid = fetch_and_verify_proof(rpc_url, 42, 5)
    print(f"Transaction proof is {'valid' if is_valid else 'invalid'}")
```

## Rust Examples

### Using tiny-keccak

```rust
use tiny_keccak::{Hasher, Keccak};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct MerkleProof {
    hashes: Vec<String>,
    transaction: serde_json::Value,
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    hex::decode(hex).expect("Invalid hex string")
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut output = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}

fn verify_keccak256_proof(proof: &MerkleProof, root: &str) -> bool {
    // Serialize and hash the transaction
    let tx_bytes = serialize_transaction(&proof.transaction);
    let mut current_hash = keccak256(&tx_bytes);
    
    println!("Starting with transaction hash: 0x{}", hex::encode(current_hash));
    
    // Process each sibling hash in the proof
    for (i, sibling_hex) in proof.hashes.iter().enumerate() {
        let sibling_bytes = hex_to_bytes(sibling_hex);
        let mut sibling_hash = [0u8; 32];
        sibling_hash.copy_from_slice(&sibling_bytes);
        
        // Sort hashes lexicographically
        let (min_hash, max_hash) = if current_hash < sibling_hash {
            (&current_hash, &sibling_hash)
        } else {
            (&sibling_hash, &current_hash)
        };
        
        println!("Level {}:", i);
        println!("  Current: 0x{}", hex::encode(current_hash));
        println!("  Sibling: {}", sibling_hex);
        
        // Concatenate and hash
        let mut combined = Vec::with_capacity(64);
        combined.extend_from_slice(min_hash);
        combined.extend_from_slice(max_hash);
        current_hash = keccak256(&combined);
        
        println!("  Result: 0x{}", hex::encode(current_hash));
    }
    
    let computed_root = format!("0x{}", hex::encode(current_hash));
    println!("Final computed root: {}", computed_root);
    println!("Expected root: {}", root);
    
    computed_root == root
}

fn serialize_transaction(tx: &serde_json::Value) -> Vec<u8> {
    // Implement according to Nimiq transaction serialization format
    // This is transaction-type specific
    todo!("Implement transaction serialization")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let rpc_url = "http://localhost:8648";
    
    // Fetch the proof
    let proof_response: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getKeccak256TransactionProof",
            "params": [42, 5],
            "id": 1
        }))
        .send()
        .await?
        .json()
        .await?;
    
    let proof: MerkleProof = serde_json::from_value(proof_response["result"].clone())?;
    
    // Fetch the root
    let root_response: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getKeccak256HistoryRoot",
            "params": [42],
            "id": 2
        }))
        .send()
        .await?
        .json()
        .await?;
    
    let root = root_response["result"].as_str().unwrap();
    
    // Verify the proof
    let is_valid = verify_keccak256_proof(&proof, root);
    
    println!("Proof verification: {}", if is_valid { "VALID" } else { "INVALID" });
    
    Ok(())
}
```

## Solidity Smart Contract Examples

### Basic Verification Contract

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/**
 * @title NimiqProofVerifier
 * @dev Verifies Nimiq Keccak256 Merkle proofs on Ethereum
 */
contract NimiqProofVerifier {
    /**
     * @dev Verify a Nimiq transaction Merkle proof
     * @param root The expected Merkle root
     * @param proof Array of sibling hashes
     * @param transactionHash The hash of the transaction being proven
     * @return bool True if the proof is valid
     */
    function verifyProof(
        bytes32 root,
        bytes32[] memory proof,
        bytes32 transactionHash
    ) public pure returns (bool) {
        bytes32 currentHash = transactionHash;
        
        for (uint256 i = 0; i < proof.length; i++) {
            bytes32 sibling = proof[i];
            
            // Sort hashes lexicographically
            if (currentHash < sibling) {
                currentHash = keccak256(abi.encodePacked(currentHash, sibling));
            } else {
                currentHash = keccak256(abi.encodePacked(sibling, currentHash));
            }
        }
        
        return currentHash == root;
    }
    
    /**
     * @dev Verify a proof and emit an event if valid
     */
    function verifyAndLog(
        bytes32 root,
        bytes32[] memory proof,
        bytes32 transactionHash
    ) public returns (bool) {
        bool isValid = verifyProof(root, proof, transactionHash);
        
        if (isValid) {
            emit ProofVerified(root, transactionHash);
        }
        
        return isValid;
    }
    
    event ProofVerified(bytes32 indexed root, bytes32 indexed transactionHash);
}
```

### Bridge Contract Example

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/**
 * @title NimiqBridge
 * @dev A simple bridge that accepts Nimiq transaction proofs
 */
contract NimiqBridge {
    // Mapping of epoch number to Merkle root
    mapping(uint32 => bytes32) public epochRoots;
    
    // Mapping to track processed transactions
    mapping(bytes32 => bool) public processedTransactions;
    
    address public owner;
    
    constructor() {
        owner = msg.sender;
    }
    
    /**
     * @dev Submit a Nimiq epoch root (only owner)
     */
    function submitEpochRoot(uint32 epochNumber, bytes32 root) external {
        require(msg.sender == owner, "Only owner can submit roots");
        require(epochRoots[epochNumber] == bytes32(0), "Root already submitted");
        
        epochRoots[epochNumber] = root;
        emit EpochRootSubmitted(epochNumber, root);
    }
    
    /**
     * @dev Process a Nimiq transaction with proof
     */
    function processTransaction(
        uint32 epochNumber,
        bytes32[] memory proof,
        bytes32 transactionHash,
        address recipient,
        uint256 amount
    ) external {
        // Check that epoch root exists
        bytes32 root = epochRoots[epochNumber];
        require(root != bytes32(0), "Epoch root not found");
        
        // Check transaction hasn't been processed
        require(!processedTransactions[transactionHash], "Transaction already processed");
        
        // Verify the proof
        require(verifyProof(root, proof, transactionHash), "Invalid proof");
        
        // Mark as processed
        processedTransactions[transactionHash] = true;
        
        // Process the transaction (e.g., mint tokens)
        // This is simplified - real implementation would parse transaction data
        emit TransactionProcessed(epochNumber, transactionHash, recipient, amount);
    }
    
    function verifyProof(
        bytes32 root,
        bytes32[] memory proof,
        bytes32 transactionHash
    ) internal pure returns (bool) {
        bytes32 currentHash = transactionHash;
        
        for (uint256 i = 0; i < proof.length; i++) {
            bytes32 sibling = proof[i];
            
            if (currentHash < sibling) {
                currentHash = keccak256(abi.encodePacked(currentHash, sibling));
            } else {
                currentHash = keccak256(abi.encodePacked(sibling, currentHash));
            }
        }
        
        return currentHash == root;
    }
    
    event EpochRootSubmitted(uint32 indexed epochNumber, bytes32 root);
    event TransactionProcessed(
        uint32 indexed epochNumber,
        bytes32 indexed transactionHash,
        address recipient,
        uint256 amount
    );
}
```

## Command Line Examples

### Using curl and jq

```bash
#!/bin/bash

RPC_URL="http://localhost:8648"
EPOCH_NUMBER=42
TX_INDEX=5

# Fetch the Keccak256 history root
echo "Fetching Keccak256 history root for epoch $EPOCH_NUMBER..."
ROOT=$(curl -s -X POST "$RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"method\": \"getKeccak256HistoryRoot\",
    \"params\": [$EPOCH_NUMBER],
    \"id\": 1
  }" | jq -r '.result')

echo "Root: $ROOT"

# Fetch the transaction proof
echo "Fetching proof for transaction $TX_INDEX in epoch $EPOCH_NUMBER..."
PROOF=$(curl -s -X POST "$RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"method\": \"getKeccak256TransactionProof\",
    \"params\": [$EPOCH_NUMBER, $TX_INDEX],
    \"id\": 2
  }" | jq '.result')

echo "Proof:"
echo "$PROOF" | jq '.'

# Extract proof hashes
HASHES=$(echo "$PROOF" | jq -r '.hashes[]')
echo "Proof hashes:"
echo "$HASHES"
```

### Batch Fetching Multiple Proofs

```bash
#!/bin/bash

RPC_URL="http://localhost:8648"
EPOCH_NUMBER=42

# Fetch all proofs for an epoch
echo "Fetching all transaction proofs for epoch $EPOCH_NUMBER..."

# First, get the number of transactions in the epoch
# (This would require additional RPC methods or querying the blockchain)

for TX_INDEX in {0..9}; do
  echo "Fetching proof for transaction $TX_INDEX..."
  
  curl -s -X POST "$RPC_URL" \
    -H "Content-Type: application/json" \
    -d "{
      \"jsonrpc\": \"2.0\",
      \"method\": \"getKeccak256TransactionProof\",
      \"params\": [$EPOCH_NUMBER, $TX_INDEX],
      \"id\": $TX_INDEX
    }" | jq -c '{index: '$TX_INDEX', proof: .result}' >> proofs_epoch_${EPOCH_NUMBER}.jsonl
done

echo "All proofs saved to proofs_epoch_${EPOCH_NUMBER}.jsonl"
```

## Testing and Debugging

### Verify Proof Locally

```javascript
// test-proof.js
const { ethers } = require('ethers');

// Example proof data (replace with actual data from RPC)
const proof = {
  hashes: [
    "0xabcd1234...",
    "0xef567890...",
    "0x23456789..."
  ],
  transaction: {
    // Transaction data
  }
};

const root = "0x1234567890abcdef...";

// Verify
const isValid = verifyKeccak256Proof(proof, root);
console.log('Proof is', isValid ? 'VALID ✓' : 'INVALID ✗');
```

### Debug Proof Step-by-Step

```javascript
function debugProof(proof, root) {
  console.log('=== Proof Verification Debug ===\n');
  
  let currentHash = ethers.utils.keccak256(
    serializeTransaction(proof.transaction)
  );
  
  console.log('Transaction Hash:', currentHash);
  console.log('Number of proof levels:', proof.hashes.length);
  console.log('Expected root:', root);
  console.log('\n--- Processing Proof ---\n');
  
  for (let i = 0; i < proof.hashes.length; i++) {
    const siblingHash = proof.hashes[i];
    const hashes = [currentHash, siblingHash].sort();
    
    console.log(`Level ${i}:`);
    console.log('  Input A:', currentHash);
    console.log('  Input B:', siblingHash);
    console.log('  Sorted:', hashes[0] === currentHash ? 'A < B' : 'B < A');
    
    currentHash = ethers.utils.keccak256(
      ethers.utils.concat([hashes[0], hashes[1]])
    );
    
    console.log('  Output:', currentHash);
    console.log();
  }
  
  console.log('--- Result ---');
  console.log('Computed root:', currentHash);
  console.log('Expected root:', root);
  console.log('Match:', currentHash === root ? 'YES ✓' : 'NO ✗');
}
```

## Additional Resources

- [Nimiq RPC Documentation](https://www.nimiq.com/developers/build/set-up-your-own-node/rpc-docs/)
- [Keccak256 History Support](./KECCAK256_HISTORY.md)
- [Ethereum Yellow Paper](https://ethereum.github.io/yellowpaper/paper.pdf)
- [ethers.js Documentation](https://docs.ethers.org/)
- [web3.py Documentation](https://web3py.readthedocs.io/)
