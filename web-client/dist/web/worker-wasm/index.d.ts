declare namespace wasm_bindgen {
	/* tslint:disable */
	/* eslint-disable */
	/**
	*/
	export enum TransactionFormat {
	  Basic = 0,
	  Extended = 1,
	}
	/**
	*/
	export enum AccountType {
	  Basic = 0,
	  Vesting = 1,
	  HTLC = 2,
	  Staking = 3,
	}
	export type PlainAccountType = "basic" | "vesting" | "htlc" | "staking";
	
	export type PlainTransactionFormat = "basic" | "extended";
	
	export interface PlainClientConfiguration {
	    networkId?: string;
	    seedNodes?: string[];
	    logLevel?: string;
	}
	
	export interface PlainBasicAccount {
	    balance: number;
	}
	
	export interface PlainVestingContract {
	    balance: number;
	    owner: string;
	    startTime: number;
	    timeStep: number;
	    stepAmount: number;
	    totalAmount: number;
	}
	
	export interface PlainHtlcContract {
	    balance: number;
	    sender: string;
	    recipient: string;
	    hashAlgorithm: string;
	    hashRoot: string;
	    hashCount: number;
	    timeout: number;
	    totalAmount: number;
	}
	
	export interface PlainStakingContract {
	    balance: number;
	    activeValidators: [string, number][];
	    currentEpochDisabledSlots: [string, number[]][];
	    previousDisabledSlots: number[];
	}
	
	export type PlainAccount = ({ type: "basic" } & PlainBasicAccount) | ({ type: "vesting" } & PlainVestingContract) | ({ type: "htlc" } & PlainHtlcContract) | ({ type: "staking" } & PlainStakingContract);
	
	/**
	 * JSON-compatible and human-readable format of a staker. E.g. delegation addresses are presented in their
	 * human-readable format.
	 */
	export interface PlainStaker {
	    /**
	     * The staker\'s active balance.
	     */
	    balance: number;
	    /**
	     * The address of the validator for which the staker is delegating its stake for. If it is not
	     * delegating to any validator, this will be set to None.
	     */
	    delegation: string | undefined;
	    /**
	     * The staker\'s inactive balance. Only released inactive balance can be withdrawn from the staking contract.
	     * Stake can only be re-delegated if the whole balance of the staker is inactive and released
	     * (or if there was no prior delegation). For inactive balance to be released, the maximum of
	     * the inactive and the validator\'s jailed periods must have passed.
	     */
	    inactiveBalance: number;
	    /**
	     * The block number from which the staker\'s `inactive_balance` becomes inactive.
	     * Stake can only effectively become inactive on the next election block. Thus, this may contain a
	     * future block height.
	     * Re-delegation requires the whole balance of the staker to be inactive and released, as well as
	     * its delegated validator to not currently be jailed.
	     */
	    inactiveFrom: number | undefined;
	    /**
	     * The block number from which the staker\'s `inactive_balance` gets released, e.g. for unstaking.
	     * Re-delegation requires the whole balance of the staker to be inactive and released, as well as
	     * its delegated validator to not currently be jailed.
	     */
	    inactiveRelease: number | undefined;
	}
	
	/**
	 * JSON-compatible and human-readable format of a validator. E.g. reward addresses and public keys are presented in
	 * their human-readable format.
	 */
	export interface PlainValidator {
	    /**
	     * The public key used to sign blocks. It is also used to retire and reactivate the validator.
	     */
	    signingPublicKey: string;
	    /**
	     * The voting public key, it is used to vote for skip and macro blocks.
	     */
	    votingPublicKey: string;
	    /**
	     * The reward address of the validator. All the block rewards are paid to this address.
	     */
	    rewardAddress: string;
	    /**
	     * Signaling field. Can be used to do chain upgrades or for any other purpose that requires
	     * validators to coordinate among themselves.
	     */
	    signalData: string | undefined;
	    /**
	     * The total stake assigned to this validator. It includes the validator deposit as well as the
	     * coins delegated to him by stakers.
	     */
	    totalStake: number;
	    /**
	     * The amount of coins deposited by this validator. The initial deposit is a fixed amount,
	     * however this value can be decremented by failing staking transactions due to fees.
	     */
	    deposit: number;
	    /**
	     * The number of stakers that are delegating to this validator.
	     */
	    numStakers: number;
	    /**
	     * An option indicating if the validator is marked as inactive. If it is, then it contains the
	     * block height at which it becomes inactive.
	     * A validator can only effectively become inactive on the next election block. Thus, this may
	     * contain a block height in the future.
	     */
	    inactiveFrom: number | undefined;
	    /**
	     * An option indicating if the validator is marked as inactive. If it is, then it contains the
	     * block height at which the inactive stake gets released and the validator can be retired.
	     */
	    inactiveRelease: number | undefined;
	    /**
	     * A flag indicating if the validator is retired.
	     */
	    retired: boolean;
	    /**
	     * An option indicating if the validator is jailed. If it is, then it contains the
	     * block height at which it became jailed.
	     * Opposed to `inactive_from`, jailing can and should take effect immediately to prevent
	     * the validator and its stakers from modifying their funds and or delegation.
	     */
	    jailedFrom: number | undefined;
	    /**
	     * An option indicating if the validator is jailed. If it is, then it contains the
	     * block height at which the jail period ends and the validator becomes interactive again.
	     */
	    jailedRelease: number | undefined;
	}
	
	/**
	 * JSON-compatible and human-readable format of blocks.
	 */
	export interface PlainBlockCommonFields {
	    /**
	     * The block\'s unique hash, used as its identifier, in HEX format.
	     */
	    hash: string;
	    /**
	     * The block\'s on-chain size, in bytes.
	     */
	    size: number;
	    /**
	     * The block\'s block height, also called block number.
	     */
	    height: number;
	    /**
	     * The batch number that the block is in.
	     */
	    batch: number;
	    /**
	     * The epoch number that the block is in.
	     */
	    epoch: number;
	    /**
	     * The timestamp of the block. It follows the Unix time and has millisecond precision.
	     */
	    timestamp: number;
	    /**
	     * The protocol version that this block is valid for.
	     */
	    version: number;
	    /**
	     * The hash of the header of the immediately preceding block (either micro or macro), in HEX format.
	     */
	    prevHash: string;
	    /**
	     * The seed of the block. This is the BLS signature of the seed of the immediately preceding
	     * block (either micro or macro) using the validator key of the block producer.
	     */
	    seed: string;
	    /**
	     * The extra data of the block, in HEX format. Up to 32 raw bytes.
	     *
	     * In the genesis block, it encodes the initial supply as a big-endian `u64`.
	     *
	     * No planned use otherwise.
	     */
	    extraData: string;
	    /**
	     * The root of the Merkle tree of the blockchain state, in HEX format. It acts as a commitment to the state.
	     */
	    stateHash: string;
	    /**
	     * The root of the Merkle tree of the body, in HEX format. It acts as a commitment to the body.
	     */
	    bodyHash: string;
	    /**
	     * A Merkle root over all of the transactions that happened in the current epoch, in HEX format.
	     */
	    historyHash: string;
	}
	
	export interface PlainMacroBlock extends PlainBlockCommonFields {
	    /**
	     * If true, this macro block is an election block finalizing an epoch.
	     */
	    isElectionBlock: boolean;
	    /**
	     * The round number this block was proposed in.
	     */
	    round: number;
	    /**
	     * The hash of the header of the preceding election macro block, in HEX format.
	     */
	    prevElectionHash: string;
	}
	
	export interface PlainMicroBlock extends PlainBlockCommonFields {}
	
	export type PlainBlock = ({ type: "macro" } & PlainMacroBlock) | ({ type: "micro" } & PlainMicroBlock);
	
	/**
	 * Information about a networking peer.
	 */
	export interface PlainPeerInfo {
	    /**
	     * Address of the peer in `Multiaddr` format
	     */
	    address: string;
	    /**
	     * Node type of the peer
	     */
	    type: 'full' | 'history' | 'light';
	}
	
	/**
	 * Describes the state of consensus of the client.
	 */
	export type ConsensusState = "connecting" | "syncing" | "established";
	
	export type PlainTransactionSenderData = PlainRawData | PlainRawData | PlainRawData;
	
	/**
	 * Placeholder struct to serialize data of transactions as hex strings in the style of the Nimiq 1.0 library.
	 */
	export type PlainTransactionRecipientData = PlainRawData | PlainVestingData | PlainHtlcData | PlainCreateValidatorData | PlainUpdateValidatorData | PlainValidatorData | PlainCreateStakerData | PlainAddStakeData | PlainUpdateStakerData | PlainSetInactiveStakeData;
	
	export interface PlainRawData {
	    raw: string;
	}
	
	export interface PlainVestingData {
	    raw: string;
	    owner: string;
	    startTime: number;
	    timeStep: number;
	    stepAmount: number;
	}
	
	export interface PlainHtlcData {
	    raw: string;
	    sender: string;
	    recipient: string;
	    hashAlgorithm: string;
	    hashRoot: string;
	    hashCount: number;
	    timeout: number;
	}
	
	export interface PlainCreateValidatorData {
	    raw: string;
	    signingKey: string;
	    votingKey: string;
	    rewardAddress: string;
	    signalData: string | undefined;
	    proofOfKnowledge: string;
	}
	
	export interface PlainUpdateValidatorData {
	    raw: string;
	    newSigningKey: string | undefined;
	    newVotingKey: string | undefined;
	    newRewardAddress: string | undefined;
	    newSignalData: string | undefined | undefined;
	    newProofOfKnowledge: string | undefined;
	}
	
	export interface PlainValidatorData {
	    raw: string;
	    validator: string;
	}
	
	export interface PlainCreateStakerData {
	    raw: string;
	    delegation: string | undefined;
	}
	
	export interface PlainAddStakeData {
	    raw: string;
	    staker: string;
	}
	
	export interface PlainUpdateStakerData {
	    raw: string;
	    newDelegation: string | undefined;
	}
	
	export interface PlainSetInactiveStakeData {
	    raw: string;
	    newInactiveBalance: number;
	}
	
	/**
	 * Placeholder struct to serialize proofs of transactions as hex strings in the style of the Nimiq 1.0 library.
	 */
	export interface PlainTransactionProof {
	    raw: string;
	}
	
	/**
	 * JSON-compatible and human-readable format of transactions. E.g. addresses are presented in their human-readable
	 * format and address types and the network are represented as strings. Data and proof are serialized as an object
	 * describing their contents (not yet implemented, only the `{ raw: string }` fallback is available).
	 */
	export interface PlainTransaction {
	    /**
	     * The transaction\'s unique hash, used as its identifier. Sometimes also called `txId`.
	     */
	    transactionHash: string;
	    /**
	     * The transaction\'s format. Nimiq transactions can have one of two formats: \"basic\" and \"extended\".
	     * Basic transactions are simple value transfers between two regular address types and cannot contain
	     * any extra data. Basic transactions can be serialized to less bytes, so take up less place on the
	     * blockchain. Extended transactions on the other hand are all other transactions: contract creations
	     * and interactions, staking transactions, transactions with exta data, etc.
	     */
	    format: PlainTransactionFormat;
	    /**
	     * The transaction\'s sender address in human-readable IBAN format.
	     */
	    sender: string;
	    /**
	     * The type of the transaction\'s sender. \"basic\" are regular private-key controlled addresses,
	     * \"vesting\" and \"htlc\" are those contract types respectively, and \"staking\" is the staking contract.
	     */
	    senderType: PlainAccountType;
	    /**
	     * The transaction\'s recipient address in human-readable IBAN format.
	     */
	    recipient: string;
	    /**
	     * The type of the transaction\'s sender. \"basic\" are regular private-key controlled addresses,
	     * \"vesting\" and \"htlc\" are those contract types respectively, and \"staking\" is the staking contract.
	     */
	    recipientType: PlainAccountType;
	    value: number;
	    /**
	     * The transaction\'s fee in luna (NIM\'s smallest unit).
	     */
	    fee: number;
	    /**
	     * The transaction\'s fee-per-byte in luna (NIM\'s smallest unit).
	     */
	    feePerByte: number;
	    /**
	     * The block height at which this transaction becomes valid. It is then valid for 7200 blocks (~2 hours).
	     */
	    validityStartHeight: number;
	    /**
	     * The network name on which this transaction is valid.
	     */
	    network: string;
	    /**
	     * Any flags that this transaction carries. `0b1 = 1` means it\'s a contract-creation transaction, `0b10 = 2`
	     * means it\'s a signalling transaction with 0 value.
	     */
	    flags: number;
	    /**
	     * The `sender_data` field serves a purpose based on the transaction\'s sender type.
	     * It is currently only used for extra information in transactions from the staking contract.
	     */
	    senderData: PlainTransactionSenderData;
	    /**
	     * The `data` field of a transaction serves different purposes based on the transaction\'s recipient type.
	     * For transactions to \"basic\" address types, this field can contain up to 64 bytes of unstructured data.
	     * For transactions that create contracts or interact with the staking contract, the format of this field
	     * must follow a fixed structure and defines the new contracts\' properties or how the staking contract is
	     * changed.
	     */
	    data: PlainTransactionRecipientData;
	    /**
	     * The `proof` field contains the signature of the eligible signer. The proof field\'s structure depends on
	     * the transaction\'s sender type. For transactions from contracts it can also contain additional structured
	     * data before the signature.
	     */
	    proof: PlainTransactionProof;
	    /**
	     * The transaction\'s serialized size in bytes. It is used to determine the fee-per-byte that this
	     * transaction pays.
	     */
	    size: number;
	    /**
	     * Encodes if the transaction is valid, meaning the signature is valid and the `data` and `proof` fields
	     * follow the correct format for the transaction\'s recipient and sender type, respectively.
	     */
	    valid: boolean;
	}
	
	/**
	 * Describes the state of a transaction as known by the client.
	 */
	export type TransactionState = "new" | "pending" | "included" | "confirmed" | "invalidated" | "expired";
	
	/**
	 * JSON-compatible and human-readable format of transactions, including details about its state in the
	 * blockchain. Contains all fields from {@link PlainTransaction}, plus additional fields such as
	 * `blockHeight` and `timestamp` if the transaction is included in the blockchain.
	 */
	export interface PlainTransactionDetails extends PlainTransaction {
	    state: TransactionState;
	    executionResult?: boolean;
	    blockHeight?: number;
	    confirmations?: number;
	    timestamp?: number;
	}
	
	/**
	 * JSON-compatible and human-readable format of transaction receipts.
	 */
	export interface PlainTransactionReceipt {
	    /**
	     * The transaction\'s unique hash, used as its identifier. Sometimes also called `txId`.
	     */
	    transactionHash: string;
	    /**
	     * The transaction\'s block height where it is included in the blockchain.
	     */
	    blockHeight: number;
	}
	
	/**
	* An object representing a Nimiq address.
	* Offers methods to parse and format addresses from and to strings.
	*/
	export class Address {
	  free(): void;
	/**
	* @param {Uint8Array} bytes
	*/
	  constructor(bytes: Uint8Array);
	/**
	* Parses an address from an {@link Address} instance or a string representation.
	*
	* Throws when an address cannot be parsed from the argument.
	* @param {string} addr
	* @returns {Address}
	*/
	  static fromAny(addr: string): Address;
	/**
	* Parses an address from a string representation, either user-friendly or hex format.
	*
	* Throws when an address cannot be parsed from the string.
	* @param {string} str
	* @returns {Address}
	*/
	  static fromString(str: string): Address;
	/**
	* Formats the address into a plain string format.
	* @returns {string}
	*/
	  toPlain(): string;
	}
	/**
	* Nimiq Albatross client that runs in browsers via WASM and is exposed to Javascript.
	*
	* ### Usage:
	*
	* ```js
	* import init, * as Nimiq from "./pkg/nimiq_web_client.js";
	*
	* init().then(async () => {
	*     const config = new Nimiq.ClientConfiguration();
	*     const client = await config.instantiateClient();
	*     // ...
	* });
	* ```
	*/
	export class Client {
	  free(): void;
	/**
	* Creates a new Client that automatically starts connecting to the network.
	* @param {PlainClientConfiguration} config
	* @returns {Promise<Client>}
	*/
	  static create(config: PlainClientConfiguration): Promise<Client>;
	/**
	* Adds an event listener for consensus-change events, such as when consensus is established or lost.
	* @param {(state: ConsensusState) => any} listener
	* @returns {Promise<number>}
	*/
	  addConsensusChangedListener(listener: (state: ConsensusState) => any): Promise<number>;
	/**
	* Adds an event listener for new blocks added to the blockchain.
	* @param {(hash: string, reason: string, reverted_blocks: string[], adopted_blocks: string[]) => any} listener
	* @returns {Promise<number>}
	*/
	  addHeadChangedListener(listener: (hash: string, reason: string, reverted_blocks: string[], adopted_blocks: string[]) => any): Promise<number>;
	/**
	* Adds an event listener for peer-change events, such as when a new peer joins, or a peer leaves.
	* @param {(peer_id: string, reason: 'joined' | 'left', peer_count: number, peer_info?: PlainPeerInfo) => any} listener
	* @returns {Promise<number>}
	*/
	  addPeerChangedListener(listener: (peer_id: string, reason: 'joined' | 'left', peer_count: number, peer_info?: PlainPeerInfo) => any): Promise<number>;
	/**
	* Adds an event listener for transactions to and from the provided addresses.
	*
	* The listener is called for transactions when they are _included_ in the blockchain.
	* @param {(transaction: PlainTransactionDetails) => any} listener
	* @param {string[]} addresses
	* @returns {Promise<number>}
	*/
	  addTransactionListener(listener: (transaction: PlainTransactionDetails) => any, addresses: string[]): Promise<number>;
	/**
	* Removes an event listener by its handle.
	* @param {number} handle
	* @returns {Promise<void>}
	*/
	  removeListener(handle: number): Promise<void>;
	/**
	* Returns the network ID that the client is connecting to.
	* @returns {Promise<number>}
	*/
	  getNetworkId(): Promise<number>;
	/**
	* Returns if the client currently has consensus with the network.
	* @returns {Promise<boolean>}
	*/
	  isConsensusEstablished(): Promise<boolean>;
	/**
	* Returns a promise that resolves when the client has established consensus with the network.
	* @returns {Promise<void>}
	*/
	  waitForConsensusEstablished(): Promise<void>;
	/**
	* Returns the block hash of the current blockchain head.
	* @returns {Promise<string>}
	*/
	  getHeadHash(): Promise<string>;
	/**
	* Returns the block number of the current blockchain head.
	* @returns {Promise<number>}
	*/
	  getHeadHeight(): Promise<number>;
	/**
	* Returns the current blockchain head block.
	* Note that the web client is a light client and does not have block bodies, i.e. no transactions.
	* @returns {Promise<PlainBlock>}
	*/
	  getHeadBlock(): Promise<PlainBlock>;
	/**
	* Fetches a block by its hash.
	*
	* Throws if the client does not have the block.
	*
	* Fetching blocks from the network is not yet available.
	* @param {string} hash
	* @returns {Promise<PlainBlock>}
	*/
	  getBlock(hash: string): Promise<PlainBlock>;
	/**
	* Fetches a block by its height (block number).
	*
	* Throws if the client does not have the block.
	*
	* Fetching blocks from the network is not yet available.
	* @param {number} height
	* @returns {Promise<PlainBlock>}
	*/
	  getBlockAt(height: number): Promise<PlainBlock>;
	/**
	* Fetches the account for the provided address from the network.
	*
	* Throws if the address cannot be parsed and on network errors.
	* @param {string} address
	* @returns {Promise<PlainAccount>}
	*/
	  getAccount(address: string): Promise<PlainAccount>;
	/**
	* Fetches the accounts for the provided addresses from the network.
	*
	* Throws if an address cannot be parsed and on network errors.
	* @param {string[]} addresses
	* @returns {Promise<PlainAccount[]>}
	*/
	  getAccounts(addresses: string[]): Promise<PlainAccount[]>;
	/**
	* Fetches the staker for the provided address from the network.
	*
	* Throws if the address cannot be parsed and on network errors.
	* @param {string} address
	* @returns {Promise<PlainStaker | undefined>}
	*/
	  getStaker(address: string): Promise<PlainStaker | undefined>;
	/**
	* Fetches the stakers for the provided addresses from the network.
	*
	* Throws if an address cannot be parsed and on network errors.
	* @param {string[]} addresses
	* @returns {Promise<(PlainStaker | undefined)[]>}
	*/
	  getStakers(addresses: string[]): Promise<(PlainStaker | undefined)[]>;
	/**
	* Fetches the validator for the provided address from the network.
	*
	* Throws if the address cannot be parsed and on network errors.
	* @param {string} address
	* @returns {Promise<PlainValidator | undefined>}
	*/
	  getValidator(address: string): Promise<PlainValidator | undefined>;
	/**
	* Fetches the validators for the provided addresses from the network.
	*
	* Throws if an address cannot be parsed and on network errors.
	* @param {string[]} addresses
	* @returns {Promise<(PlainValidator | undefined)[]>}
	*/
	  getValidators(addresses: string[]): Promise<(PlainValidator | undefined)[]>;
	/**
	* Sends a transaction to the network and returns {@link PlainTransactionDetails}.
	*
	* Throws in case of network errors.
	* @param {PlainTransaction | string} transaction
	* @returns {Promise<PlainTransactionDetails>}
	*/
	  sendTransaction(transaction: PlainTransaction | string): Promise<PlainTransactionDetails>;
	/**
	* Fetches the transaction details for the given transaction hash.
	* @param {string} hash
	* @returns {Promise<PlainTransactionDetails>}
	*/
	  getTransaction(hash: string): Promise<PlainTransactionDetails>;
	/**
	* This function is used to query the network for transaction receipts from and to a
	* specific address, that have been included in the chain.
	*
	* The obtained receipts are _not_ verified before being returned.
	*
	* Up to a `limit` number of transaction receipts are returned from newest to oldest.
	* If the network does not have at least `min_peers` to query, then an error is returned.
	* @param {string} address
	* @param {number | undefined} [limit]
	* @param {number | undefined} [min_peers]
	* @returns {Promise<PlainTransactionReceipt[]>}
	*/
	  getTransactionReceiptsByAddress(address: string, limit?: number, min_peers?: number): Promise<PlainTransactionReceipt[]>;
	/**
	* This function is used to query the network for transactions from and to a specific
	* address, that have been included in the chain.
	*
	* The obtained transactions are verified before being returned.
	*
	* Up to a `limit` number of transactions are returned from newest to oldest.
	* If the network does not have at least `min_peers` to query, then an error is returned.
	* @param {string} address
	* @param {number | undefined} [since_block_height]
	* @param {PlainTransactionDetails[] | undefined} [known_transaction_details]
	* @param {number | undefined} [limit]
	* @param {number | undefined} [min_peers]
	* @returns {Promise<PlainTransactionDetails[]>}
	*/
	  getTransactionsByAddress(address: string, since_block_height?: number, known_transaction_details?: PlainTransactionDetails[], limit?: number, min_peers?: number): Promise<PlainTransactionDetails[]>;
	}
	/**
	* Use this to provide initialization-time configuration to the Client.
	* This is a simplified version of the configuration that is used for regular nodes,
	* since not all configuration knobs are available when running inside a browser.
	*/
	export class ClientConfiguration {
	  free(): void;
	}
	/**
	*/
	export class Policy {
	  free(): void;
	/**
	* Returns the epoch number at a given block number (height).
	* @param {number} block_number
	* @returns {number}
	*/
	  static epochAt(block_number: number): number;
	/**
	* Returns the epoch index at a given block number. The epoch index is the number of a block relative
	* to the epoch it is in. For example, the first block of any epoch always has an epoch index of 0.
	* @param {number} block_number
	* @returns {number}
	*/
	  static epochIndexAt(block_number: number): number;
	/**
	* Returns the batch number at a given `block_number` (height)
	* @param {number} block_number
	* @returns {number}
	*/
	  static batchAt(block_number: number): number;
	/**
	* Returns the batch index at a given block number. The batch index is the number of a block relative
	* to the batch it is in. For example, the first block of any batch always has an batch index of 0.
	* @param {number} block_number
	* @returns {number}
	*/
	  static batchIndexAt(block_number: number): number;
	/**
	* Returns the number (height) of the next election macro block after a given block number (height).
	* @param {number} block_number
	* @returns {number}
	*/
	  static electionBlockAfter(block_number: number): number;
	/**
	* Returns the block number (height) of the preceding election macro block before a given block number (height).
	* If the given block number is an election macro block, it returns the election macro block before it.
	* @param {number} block_number
	* @returns {number}
	*/
	  static electionBlockBefore(block_number: number): number;
	/**
	* Returns the block number (height) of the last election macro block at a given block number (height).
	* If the given block number is an election macro block, then it returns that block number.
	* @param {number} block_number
	* @returns {number}
	*/
	  static lastElectionBlock(block_number: number): number;
	/**
	* Returns a boolean expressing if the block at a given block number (height) is an election macro block.
	* @param {number} block_number
	* @returns {boolean}
	*/
	  static isElectionBlockAt(block_number: number): boolean;
	/**
	* Returns the block number (height) of the next macro block after a given block number (height).
	* @param {number} block_number
	* @returns {number}
	*/
	  static macroBlockAfter(block_number: number): number;
	/**
	* Returns the block number (height) of the preceding macro block before a given block number (height).
	* If the given block number is a macro block, it returns the macro block before it.
	* @param {number} block_number
	* @returns {number}
	*/
	  static macroBlockBefore(block_number: number): number;
	/**
	* Returns the block number (height) of the last macro block at a given block number (height).
	* If the given block number is a macro block, then it returns that block number.
	* @param {number} block_number
	* @returns {number}
	*/
	  static lastMacroBlock(block_number: number): number;
	/**
	* Returns a boolean expressing if the block at a given block number (height) is a macro block.
	* @param {number} block_number
	* @returns {boolean}
	*/
	  static isMacroBlockAt(block_number: number): boolean;
	/**
	* Returns a boolean expressing if the block at a given block number (height) is a micro block.
	* @param {number} block_number
	* @returns {boolean}
	*/
	  static isMicroBlockAt(block_number: number): boolean;
	/**
	* Returns the block number of the first block of the given epoch (which is always a micro block).
	* If the index is out of bounds, None is returned
	* @param {number} epoch
	* @returns {number | undefined}
	*/
	  static firstBlockOf(epoch: number): number | undefined;
	/**
	* Returns the block number of the first block of the given batch (which is always a micro block).
	* If the index is out of bounds, None is returned
	* @param {number} batch
	* @returns {number | undefined}
	*/
	  static firstBlockOfBatch(batch: number): number | undefined;
	/**
	* Returns the block number of the election macro block of the given epoch (which is always the last block).
	* If the index is out of bounds, None is returned
	* @param {number} epoch
	* @returns {number | undefined}
	*/
	  static electionBlockOf(epoch: number): number | undefined;
	/**
	* Returns the block number of the macro block (checkpoint or election) of the given batch (which
	* is always the last block).
	* If the index is out of bounds, None is returned
	* @param {number} batch
	* @returns {number | undefined}
	*/
	  static macroBlockOf(batch: number): number | undefined;
	/**
	* Returns a boolean expressing if the batch at a given block number (height) is the first batch
	* of the epoch.
	* @param {number} block_number
	* @returns {boolean}
	*/
	  static firstBatchOfEpoch(block_number: number): boolean;
	/**
	* Returns the block height for the last block of the reporting window of a given block number.
	* Note: This window is meant for reporting malicious behaviour (aka `jailable` behaviour).
	* @param {number} block_number
	* @returns {number}
	*/
	  static lastBlockOfReportingWindow(block_number: number): number;
	/**
	* Returns the first block after the reporting window of a given block number has ended.
	* @param {number} block_number
	* @returns {number}
	*/
	  static blockAfterReportingWindow(block_number: number): number;
	/**
	* Returns the first block after the jail period of a given block number has ended.
	* @param {number} block_number
	* @returns {number}
	*/
	  static blockAfterJail(block_number: number): number;
	/**
	* Returns the supply at a given time (as Unix time) in Lunas (1 NIM = 100,000 Lunas). It is
	* calculated using the following formula:
	* Supply (t) = Genesis_supply + Initial_supply_velocity / Supply_decay * (1 - e^(- Supply_decay * t))
	* Where e is the exponential function, t is the time in milliseconds since the genesis block and
	* Genesis_supply is the supply at the genesis of the Nimiq 2.0 chain.
	* @param {bigint} genesis_supply
	* @param {bigint} genesis_time
	* @param {bigint} current_time
	* @returns {bigint}
	*/
	  static supplyAt(genesis_supply: bigint, genesis_time: bigint, current_time: bigint): bigint;
	/**
	* Returns the percentage reduction that should be applied to the rewards due to a delayed batch.
	* This function returns a float in the range [0, 1]
	* I.e 1 means that the full rewards should be given, whereas 0.5 means that half of the rewards should be given
	* The input to this function is the batch delay, in milliseconds
	* The function is: [(1 - MINIMUM_REWARDS_PERCENTAGE) * e ^(-BLOCKS_DELAY_DECAY * t^2)] + MINIMUM_REWARDS_PERCENTAGE
	* @param {bigint} delay
	* @returns {number}
	*/
	  static batchDelayPenalty(delay: bigint): number;
	/**
	* How many batches constitute an epoch
	*/
	  static readonly BATCHES_PER_EPOCH: number;
	/**
	* The slope of the exponential decay used to punish validators for not producing block in time
	*/
	  static readonly BLOCKS_DELAY_DECAY: number;
	/**
	* Length of a batch including the macro block
	*/
	  static readonly BLOCKS_PER_BATCH: number;
	/**
	* Length of an epoch including the election block
	*/
	  static readonly BLOCKS_PER_EPOCH: number;
	/**
	* The timeout in milliseconds for a validator to produce a block (2s)
	*/
	  static readonly BLOCK_PRODUCER_TIMEOUT: bigint;
	/**
	* The optimal time in milliseconds between blocks (1s)
	*/
	  static readonly BLOCK_SEPARATION_TIME: bigint;
	/**
	* The maximum size of the BLS public key cache.
	*/
	  static readonly BLS_CACHE_MAX_CAPACITY: number;
	/**
	* This is the address for the coinbase. Note that this is not a real account, it is just the
	* address we use to denote that some coins originated from a coinbase event.
	*/
	  static readonly COINBASE_ADDRESS: string;
	/**
	* Calculates f+1 slots which is the minimum number of slots necessary to be guaranteed to have at
	* least one honest slots. That's because from a total of 3f+1 slots at most f will be malicious.
	* It is calculated as `ceil(SLOTS/3)` and we use the formula `ceil(x/y) = (x+y-1)/y` for the
	* ceiling division.
	*/
	  static readonly F_PLUS_ONE: number;
	/**
	* Genesis block number
	*/
	  static readonly GENESIS_BLOCK_NUMBER: number;
	/**
	* Maximum size of history chunks.
	* 25 MB.
	*/
	  static readonly HISTORY_CHUNKS_MAX_SIZE: bigint;
	/**
	* This is the number of Lunas (1 NIM = 100,000 Lunas) created by millisecond at the genesis of the
	* Nimiq 2.0 chain. The velocity then decreases following the formula:
	* Supply_velocity (t) = Initial_supply_velocity * e^(- Supply_decay * t)
	* Where e is the exponential function and t is the time in milliseconds since the genesis block.
	*/
	  static readonly INITIAL_SUPPLY_VELOCITY: number;
	/**
	* The number of epochs a validator is put in jail for. The jailing only happens for severe offenses.
	*/
	  static readonly JAIL_EPOCHS: number;
	/**
	* The maximum allowed size, in bytes, for a micro block body.
	*/
	  static readonly MAX_SIZE_MICRO_BODY: number;
	/**
	* The minimum rewards percentage that we allow
	*/
	  static readonly MINIMUM_REWARDS_PERCENTAGE: number;
	/**
	* Minimum number of epochs that the ChainStore will store fully
	*/
	  static readonly MIN_EPOCHS_STORED: number;
	/**
	* Number of available validator slots. Note that a single validator may own several validator slots.
	*/
	  static readonly SLOTS: number;
	/**
	* This is the address for the staking contract.
	*/
	  static readonly STAKING_CONTRACT_ADDRESS: string;
	/**
	* Maximum size of accounts trie chunks.
	*/
	  static readonly STATE_CHUNKS_MAX_SIZE: number;
	/**
	* The supply decay is a constant that is calculated so that the supply velocity decreases at a
	* steady 1.47% per year.
	*/
	  static readonly SUPPLY_DECAY: number;
	/**
	* Tendermint's timeout delta, in milliseconds.
	*
	* See <https://arxiv.org/abs/1807.04938v3> for more information.
	*/
	  static readonly TENDERMINT_TIMEOUT_DELTA: bigint;
	/**
	* Tendermint's initial timeout, in milliseconds.
	*
	* See <https://arxiv.org/abs/1807.04938v3> for more information.
	*/
	  static readonly TENDERMINT_TIMEOUT_INIT: bigint;
	/**
	* The maximum drift, in milliseconds, that is allowed between any block's timestamp and the node's
	* system time. We only care about drifting to the future.
	*/
	  static readonly TIMESTAMP_MAX_DRIFT: bigint;
	/**
	* Total supply in units.
	*/
	  static readonly TOTAL_SUPPLY: bigint;
	/**
	* Number of batches a transaction is valid with Albatross consensus.
	*/
	  static readonly TRANSACTION_VALIDITY_WINDOW: number;
	/**
	* Number of blocks a transaction is valid with Albatross consensus.
	*/
	  static readonly TRANSACTION_VALIDITY_WINDOW_BLOCKS: number;
	/**
	* Calculates 2f+1 slots which is the minimum number of slots necessary to produce a macro block,
	* a skip block and other actions.
	* It is also the minimum number of slots necessary to be guaranteed to have a majority of honest
	* slots. That's because from a total of 3f+1 slots at most f will be malicious. If in a group of
	* 2f+1 slots we have f malicious ones (which is the worst case scenario), that still leaves us
	* with f+1 honest slots. Which is more than the f slots that are not in this group (which must all
	* be honest).
	* It is calculated as `ceil(SLOTS*2/3)` and we use the formula `ceil(x/y) = (x+y-1)/y` for the
	* ceiling division.
	*/
	  static readonly TWO_F_PLUS_ONE: number;
	/**
	* The deposit necessary to create a validator in Lunas (1 NIM = 100,000 Lunas).
	* A validator is someone who actually participates in block production. They are akin to miners
	* in proof-of-work.
	*/
	  static readonly VALIDATOR_DEPOSIT: bigint;
	/**
	* The current version number of the protocol. Changing this always results in a hard fork.
	*/
	  static readonly VERSION: number;
	}
	/**
	* Transactions describe a transfer of value, usually from the sender to the recipient.
	* However, transactions can also have no value, when they are used to _signal_ a change in the staking contract.
	*
	* Transactions can be used to create contracts, such as vesting contracts and HTLCs.
	*
	* Transactions require a valid signature proof over their serialized content.
	* Furthermore, transactions are only valid for 2 hours after their validity-start block height.
	*/
	export class Transaction {
	  free(): void;
	/**
	* Creates a new unsigned transaction that transfers `value` amount of luna (NIM's smallest unit)
	* from the sender to the recipient, where both sender and recipient can be any account type,
	* and custom extra data can be added to the transaction.
	*
	* ### Basic transactions
	* If both the sender and recipient types are omitted or `0` and both data and flags are empty,
	* a smaller basic transaction is created.
	*
	* ### Extended transactions
	* If no flags are given, but sender type is not basic (`0`) or data is set, an extended
	* transaction is created.
	*
	* ### Contract creation transactions
	* To create a new vesting or HTLC contract, set `flags` to `0b1` and specify the contract
	* type as the `recipient_type`: `1` for vesting, `2` for HTLC. The `data` bytes must have
	* the correct format of contract creation data for the respective contract type.
	*
	* ### Signaling transactions
	* To interact with the staking contract, signaling transaction are often used to not
	* transfer any value, but to simply _signal_ a state change instead, such as changing one's
	* delegation from one validator to another. To create such a transaction, set `flags` to `
	* 0b10` and populate the `data` bytes accordingly.
	*
	* The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
	*
	* Throws when an account type is unknown, the numbers given for value and fee do not fit
	* within a u64 or the networkId is unknown. Also throws when no data or recipient type is
	* given for contract creation transactions, or no data is given for signaling transactions.
	* @param {Address} sender
	* @param {number | undefined} sender_type
	* @param {Uint8Array | undefined} sender_data
	* @param {Address} recipient
	* @param {number | undefined} recipient_type
	* @param {Uint8Array | undefined} recipient_data
	* @param {bigint} value
	* @param {bigint} fee
	* @param {number | undefined} flags
	* @param {number} validity_start_height
	* @param {number} network_id
	*/
	  constructor(sender: Address, sender_type: number | undefined, sender_data: Uint8Array | undefined, recipient: Address, recipient_type: number | undefined, recipient_data: Uint8Array | undefined, value: bigint, fee: bigint, flags: number | undefined, validity_start_height: number, network_id: number);
	/**
	* Computes the transaction's hash, which is used as its unique identifier on the blockchain.
	* @returns {string}
	*/
	  hash(): string;
	/**
	* Verifies that a transaction has valid properties and a valid signature proof.
	* Optionally checks if the transaction is valid on the provided network.
	*
	* **Throws with any transaction validity error.** Returns without exception if the transaction is valid.
	*
	* Throws when the given networkId is unknown.
	* @param {number | undefined} [network_id]
	*/
	  verify(network_id?: number): void;
	/**
	* Tests if the transaction is valid at the specified block height.
	* @param {number} block_height
	* @returns {boolean}
	*/
	  isValidAt(block_height: number): boolean;
	/**
	* Returns the address of the contract that is created with this transaction.
	* @returns {Address}
	*/
	  getContractCreationAddress(): Address;
	/**
	* Serializes the transaction's content to be used for creating its signature.
	* @returns {Uint8Array}
	*/
	  serializeContent(): Uint8Array;
	/**
	* Serializes the transaction to a byte array.
	* @returns {Uint8Array}
	*/
	  serialize(): Uint8Array;
	/**
	* Serializes the transaction into a HEX string.
	* @returns {string}
	*/
	  toHex(): string;
	/**
	* Creates a JSON-compatible plain object representing the transaction.
	* @returns {PlainTransaction}
	*/
	  toPlain(): PlainTransaction;
	/**
	* Parses a transaction from a {@link Transaction} instance, a plain object, or a serialized
	* string representation.
	*
	* Throws when a transaction cannot be parsed from the argument.
	* @param {PlainTransaction | string} tx
	* @returns {Transaction}
	*/
	  static fromAny(tx: PlainTransaction | string): Transaction;
	/**
	* Parses a transaction from a plain object.
	*
	* Throws when a transaction cannot be parsed from the argument.
	* @param {PlainTransaction} plain
	* @returns {Transaction}
	*/
	  static fromPlain(plain: PlainTransaction): Transaction;
	/**
	* The transaction's data as a byte array.
	*/
	  data: Uint8Array;
	/**
	* The transaction's fee in luna (NIM's smallest unit).
	*/
	  readonly fee: bigint;
	/**
	* The transaction's fee per byte in luna (NIM's smallest unit).
	*/
	  readonly feePerByte: number;
	/**
	* The transaction's flags: `0b1` = contract creation, `0b10` = signaling.
	*/
	  readonly flags: number;
	/**
	* The transaction's {@link TransactionFormat}.
	*/
	  readonly format: TransactionFormat;
	/**
	* The transaction's network ID.
	*/
	  readonly networkId: number;
	/**
	* The transaction's signature proof as a byte array.
	*/
	  proof: Uint8Array;
	/**
	* The transaction's recipient address.
	*/
	  readonly recipient: Address;
	/**
	* The transaction's recipient {@link AccountType}.
	*/
	  readonly recipientType: AccountType;
	/**
	* The transaction's sender address.
	*/
	  readonly sender: Address;
	/**
	* The transaction's sender data as a byte array.
	*/
	  readonly senderData: Uint8Array;
	/**
	* The transaction's sender {@link AccountType}.
	*/
	  readonly senderType: AccountType;
	/**
	* The transaction's byte size.
	*/
	  readonly serializedSize: number;
	/**
	* The transaction's validity-start height. The transaction is valid for 2 hours after this block height.
	*/
	  readonly validityStartHeight: number;
	/**
	* The transaction's value in luna (NIM's smallest unit).
	*/
	  readonly value: bigint;
	}
	
}

declare type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

declare interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly __wbg_policy_free: (a: number) => void;
  readonly policy_transaction_validity_window: () => number;
  readonly policy_transaction_validity_window_blocks: () => number;
  readonly policy_batches_per_epoch: () => number;
  readonly policy_blocks_per_batch: () => number;
  readonly policy_blocks_per_epoch: () => number;
  readonly policy_genesis_block_number: () => number;
  readonly policy_tendermint_timeout_init: () => number;
  readonly policy_tendermint_timeout_delta: () => number;
  readonly policy_state_chunks_max_size: () => number;
  readonly policy_epochAt: (a: number) => number;
  readonly policy_epochIndexAt: (a: number) => number;
  readonly policy_batchAt: (a: number) => number;
  readonly policy_batchIndexAt: (a: number) => number;
  readonly policy_electionBlockAfter: (a: number) => number;
  readonly policy_electionBlockBefore: (a: number) => number;
  readonly policy_lastElectionBlock: (a: number) => number;
  readonly policy_isElectionBlockAt: (a: number) => number;
  readonly policy_macroBlockAfter: (a: number) => number;
  readonly policy_macroBlockBefore: (a: number) => number;
  readonly policy_lastMacroBlock: (a: number) => number;
  readonly policy_isMacroBlockAt: (a: number) => number;
  readonly policy_isMicroBlockAt: (a: number) => number;
  readonly policy_firstBlockOf: (a: number, b: number) => void;
  readonly policy_firstBlockOfBatch: (a: number, b: number) => void;
  readonly policy_electionBlockOf: (a: number, b: number) => void;
  readonly policy_macroBlockOf: (a: number, b: number) => void;
  readonly policy_firstBatchOfEpoch: (a: number) => number;
  readonly policy_lastBlockOfReportingWindow: (a: number) => number;
  readonly policy_blockAfterReportingWindow: (a: number) => number;
  readonly policy_blockAfterJail: (a: number) => number;
  readonly policy_supplyAt: (a: number, b: number, c: number) => number;
  readonly policy_batchDelayPenalty: (a: number) => number;
  readonly policy_wasm_staking_contract_address: (a: number) => void;
  readonly policy_wasm_coinbase_address: (a: number) => void;
  readonly policy_wasm_max_size_micro_body: () => number;
  readonly policy_wasm_slots: () => number;
  readonly policy_wasm_two_f_plus_one: () => number;
  readonly policy_wasm_f_plus_one: () => number;
  readonly policy_wasm_block_producer_timeout: () => number;
  readonly policy_wasm_block_separation_time: () => number;
  readonly policy_wasm_min_epochs_stored: () => number;
  readonly policy_wasm_timestamp_max_drift: () => number;
  readonly policy_wasm_blocks_delay_decay: () => number;
  readonly policy_wasm_minimum_rewards_percentage: () => number;
  readonly policy_wasm_validator_deposit: () => number;
  readonly policy_wasm_jail_epochs: () => number;
  readonly policy_wasm_total_supply: () => number;
  readonly policy_wasm_initial_supply_velocity: () => number;
  readonly policy_wasm_supply_decay: () => number;
  readonly policy_wasm_bls_cache_max_capacity: () => number;
  readonly policy_wasm_history_chunks_max_size: () => number;
  readonly policy_wasm_version: () => number;
  readonly __wbg_address_free: (a: number) => void;
  readonly address_new: (a: number, b: number, c: number) => void;
  readonly address_fromAny: (a: number, b: number) => void;
  readonly address_fromString: (a: number, b: number, c: number) => void;
  readonly address_toPlain: (a: number, b: number) => void;
  readonly __wbg_clientconfiguration_free: (a: number) => void;
  readonly __wbg_client_free: (a: number) => void;
  readonly client_create: (a: number) => number;
  readonly client_addConsensusChangedListener: (a: number, b: number) => number;
  readonly client_addHeadChangedListener: (a: number, b: number) => number;
  readonly client_addPeerChangedListener: (a: number, b: number) => number;
  readonly client_addTransactionListener: (a: number, b: number, c: number) => number;
  readonly client_removeListener: (a: number, b: number) => number;
  readonly client_getNetworkId: (a: number) => number;
  readonly client_isConsensusEstablished: (a: number) => number;
  readonly client_waitForConsensusEstablished: (a: number) => number;
  readonly client_getHeadHash: (a: number) => number;
  readonly client_getHeadHeight: (a: number) => number;
  readonly client_getHeadBlock: (a: number) => number;
  readonly client_getBlock: (a: number, b: number, c: number) => number;
  readonly client_getBlockAt: (a: number, b: number) => number;
  readonly client_getAccount: (a: number, b: number) => number;
  readonly client_getAccounts: (a: number, b: number) => number;
  readonly client_getStaker: (a: number, b: number) => number;
  readonly client_getStakers: (a: number, b: number) => number;
  readonly client_getValidator: (a: number, b: number) => number;
  readonly client_getValidators: (a: number, b: number) => number;
  readonly client_sendTransaction: (a: number, b: number) => number;
  readonly client_getTransaction: (a: number, b: number, c: number) => number;
  readonly client_getTransactionReceiptsByAddress: (a: number, b: number, c: number, d: number, e: number) => number;
  readonly client_getTransactionsByAddress: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
  readonly __wbg_transaction_free: (a: number) => void;
  readonly transaction_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number) => void;
  readonly transaction_hash: (a: number, b: number) => void;
  readonly transaction_verify: (a: number, b: number, c: number) => void;
  readonly transaction_isValidAt: (a: number, b: number) => number;
  readonly transaction_getContractCreationAddress: (a: number) => number;
  readonly transaction_serializeContent: (a: number, b: number) => void;
  readonly transaction_serialize: (a: number, b: number) => void;
  readonly transaction_format: (a: number) => number;
  readonly transaction_sender: (a: number) => number;
  readonly transaction_senderType: (a: number) => number;
  readonly transaction_recipient: (a: number) => number;
  readonly transaction_recipientType: (a: number) => number;
  readonly transaction_value: (a: number) => number;
  readonly transaction_fee: (a: number) => number;
  readonly transaction_feePerByte: (a: number) => number;
  readonly transaction_validityStartHeight: (a: number) => number;
  readonly transaction_networkId: (a: number) => number;
  readonly transaction_flags: (a: number) => number;
  readonly transaction_data: (a: number, b: number) => void;
  readonly transaction_set_data: (a: number, b: number, c: number) => void;
  readonly transaction_senderData: (a: number, b: number) => void;
  readonly transaction_proof: (a: number, b: number) => void;
  readonly transaction_set_proof: (a: number, b: number, c: number) => void;
  readonly transaction_serializedSize: (a: number) => number;
  readonly transaction_toHex: (a: number, b: number) => void;
  readonly transaction_toPlain: (a: number, b: number) => void;
  readonly transaction_fromAny: (a: number, b: number) => void;
  readonly transaction_fromPlain: (a: number, b: number) => void;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_export_2: WebAssembly.Table;
  readonly wasm_bindgen__convert__closures__invoke0_mut__hef811265daa3a595: (a: number, b: number) => void;
  readonly wasm_bindgen__convert__closures__invoke0_mut__h44c50f1b99f904ff: (a: number, b: number) => void;
  readonly wasm_bindgen__convert__closures__invoke1_mut__h21b0590f73469ed7: (a: number, b: number, c: number) => void;
  readonly _dyn_core__ops__function__Fn__A____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h5ecdb55d58a32349: (a: number, b: number, c: number) => void;
  readonly wasm_bindgen__convert__closures__invoke1_mut__he76e9651c19926b4: (a: number, b: number, c: number) => void;
  readonly wasm_bindgen__convert__closures__invoke0_mut__h6ead74bc406c3080: (a: number, b: number) => void;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly wasm_bindgen__convert__closures__invoke2_mut__h2ff626fa09ccddc4: (a: number, b: number, c: number, d: number) => void;
  readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
}

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {InitInput | Promise<InitInput>} module_or_path
*
* @returns {Promise<InitOutput>}
*/
declare function wasm_bindgen (module_or_path?: InitInput | Promise<InitInput>): Promise<InitOutput>;
