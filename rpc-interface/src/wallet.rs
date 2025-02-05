use async_trait::async_trait;
use nimiq_keys::{Address, Ed25519PublicKey, Ed25519Signature};

use crate::types::{RPCResult, ReturnAccount, ReturnSignature};

#[nimiq_jsonrpc_derive::proxy(name = "WalletProxy", rename_all = "camelCase")]
#[async_trait]
pub trait WalletInterface {
    type Error;

    /// Imports an account using its private key (in hexadecimal format) and optionally locks it with a passphrase.
    ///
    /// **Parameters**:  
    /// - `key_data` (`string`): The private key of the account to be imported, provided in hexadecimal format.  
    /// - `passphrase` (optional, `string`): An optional passphrase to lock the imported account for added security.
    ///
    /// **Returns**:  
    /// - `data`:  
    ///   - `address` (`Address`): The imported account's address.  
    ///
    /// **Example Response**:  
    /// ```json
    /// {
    ///   "data": {
    ///     "address": "string"
    ///   },
    ///   "metadata": null
    /// }
    /// ```
    async fn import_raw_key(
        &mut self,
        key_data: String,
        passphrase: Option<String>,
    ) -> RPCResult<Address, (), Self::Error>;

    /// Checks if an account with the specified address has been imported.
    ///
    /// **Parameters**:  
    /// - `address` (`Address`): The address of the account to check, provided in its hexadecimal format..
    ///
    /// **Returns**:  
    /// - `true` if the account is imported.  
    /// - `false` if the account is not imported.
    // `nimiq_jsonrpc_derive::proxy` requires the receiver type to be a mutable reference.
    #[allow(clippy::wrong_self_convention)]
    async fn is_account_imported(&mut self, address: Address) -> RPCResult<bool, (), Self::Error>;

    /// Returns the list of accounts that have been imported, including address, balance, and additional account fields.
    ///
    /// **Parameters**: None. This method does not require any input.
    ///
    /// **Returns**:  
    /// A list of imported accounts, each containing:
    /// - `address` (`Address`): The hexadecimal address of the account.
    /// - `balance` (`Coin`): The account's balance in Lunas (1 NIM = 100,000 Lunas).
    /// - `account_additional_fields` (`AccountAdditionalFields`): The type of account (e.g., `"basic"` for standard accounts).
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "address": "string",
    ///       "balance": number,
    ///       "account_additional_fields": "string"
    ///     }
    ///   ],
    ///   "metadata": null
    /// }
    /// ```

    async fn list_accounts(&mut self) -> RPCResult<Vec<Address>, (), Self::Error>;

    /// Locks the specified account, preventing further usage until it is unlocked.
    ///
    /// **Parameters**:  
    /// - `address` (`Address`): The Nimiq address of the account to lock. The address should be in the human-readable Nimiq format (e.g., `NQ76 NEXT NKMV FFFF CMYG Q7D9 VNM5 KKXB DA51`).
    ///
    /// **Returns**:  
    /// - This method does not return any data. A successful call will result in an empty response.
    ///
    /// **Additional Notes**:  
    /// - To verify if an account is locked or unlocked, use the `isAccountUnlocked` method.
    async fn lock_account(&mut self, address: Address) -> RPCResult<(), (), Self::Error>;

    /// Creates a new account and stores it securely. The account is created in a locked state and must be unlocked before use.
    ///
    /// **Parameters**:  
    /// - `passphrase` (optional, `string`): The encryption passphrase to secure the account. If omitted, the account will be stored without a passphrase.
    ///
    /// **Returns**:  
    /// - `data`:  
    ///   - `address` (`Addresss`): The generated Nimiq address.  
    ///   - `public_key` (`Ed25519PublicKey`): The public key associated with the account.  
    ///   - `private_key` (`PrivateKey`): The private key of the account (hidden for security reasons).  
    ///
    /// **Example Response**:  
    /// ```json
    /// {
    ///   "data": {
    ///     "address": "string",
    ///     "public_key": "string",
    ///     "private_key": "hidden"
    ///   },
    ///   "metadata": null
    /// }
    /// ```
    async fn create_account(
        &mut self,
        passphrase: Option<String>,
    ) -> RPCResult<ReturnAccount, (), Self::Error>;

    /// Unlocks the specified account for usage, allowing operations such as transactions.
    ///
    /// **Parameters**:  
    /// - `address` (`string`, required): The Nimiq address of the account to unlock.
    /// - `passphrase` (`string`, optional): The passphrase used to secure the account. Required if the account is locked with a passphrase.
    ///
    /// **Returns**:  
    /// - `boolean`:  
    ///   - `true`: If the account was successfully unlocked.  
    ///   - `false`: If unlocking the account failed (e.g., incorrect passphrase or the account was not locked).  
    ///
    /// **Additional Notes**:  
    /// - Use the `isUnlocked` method to verify the unlock status of the account.
    ///
    /// **Example Response (Successful Unlock)**:  
    /// ```json
    /// {
    ///   "data": true,
    ///   "metadata": null
    /// }
    /// ```
    async fn unlock_account(
        &mut self,
        address: Address,
        passphrase: Option<String>,
        duration: Option<u64>,
    ) -> RPCResult<bool, (), Self::Error>;

    /// Removes an imported account from the system.
    ///
    /// **Irreversible Action**: Once an account is removed, it cannot be recovered unless the private key is securely backed up.  
    ///
    /// **Parameters**:  
    /// - `address` (`string`, required): The Nimiq address of the account to be removed.
    ///
    /// **Returns**:  
    /// - `boolean`:  
    ///   - `true`: If the account was successfully removed.  
    ///   - `false`: If the account could not be removed.  
    ///
    /// **Example Response**:  
    /// ```json
    /// {
    ///   "data": true,
    ///   "metadata": null
    /// }
    /// ```
    async fn remove_account(&mut self, address: Address) -> RPCResult<bool, (), Self::Error>;

    /// Checks if the specified account is currently unlocked.
    ///
    /// **Parameters**:  
    /// - `address` (`string`, required): The Nimiq address of the account to check. The address should be in the human-readable Nimiq format.
    ///
    /// **Returns**:  
    /// - `boolean`:  
    ///   - `true`: If the account is currently unlocked.  
    ///   - `false`: If the account is locked.  
    ///
    /// **Notes**:  
    /// - Use this method to verify the status of an account before attempting operations that require it to be unlocked.  
    /// - Accounts that are locked cannot perform operations like sending transactions until they are unlocked.  
    ///
    /// **Example Response**:  
    /// ```json
    /// {
    ///   "data": true,
    ///   "metadata": null
    /// }
    /// ```
    // `nimiq_jsonrpc_derive::proxy` requires the receiver type to be a mutable reference.
    #[allow(clippy::wrong_self_convention)]
    async fn is_account_unlocked(&mut self, address: Address) -> RPCResult<bool, (), Self::Error>;

    /// Signs a message using the specified account. The account used for signing must be unlocked.
    ///
    /// **Parameters**:  
    /// - `message` (`string`, required): The message to sign. This can either be raw data or a hexadecimal string.  
    /// - `address` (`string`, required): The Nimiq address of the account to use for signing.  
    /// - `passphrase` (`string`, optional): The encryption passphrase for unlocking the account, if required.  
    /// - `isHex` (`boolean`, optional): Indicates whether the message is in hexadecimal format (`true`) or raw string format (`false`). Defaults to `false`.  
    ///
    /// **Returns**:  
    /// - `object`: An object containing the following fields:  
    ///   - `publicKey` (`Ed25519PublicKey`): The public key associated with the account used for signing.  
    ///   - `signature` (`object`): The cryptographic signature of the message, consisting of:  
    ///     - `R` (`string`): The first component of the Ed25519 signature.  
    ///     - `s` (`string`): The second component of the Ed25519 signature.  
    ///
    /// **Notes**:  
    /// - If `isHex` is set to `true`, the `message` must be a valid hexadecimal string; otherwise, an error will be returned.  
    ///
    /// **Example Response**:  
    /// ```json
    /// {
    ///   "data": {
    ///     "publicKey": "string",
    ///     "signature": {
    ///       "R": "string",
    ///       "s": "string"
    ///     }
    ///   },
    ///   "metadata": null
    /// }
    /// ```
    async fn sign(
        &mut self,
        message: String,
        address: Address,
        passphrase: Option<String>,
        is_hex: bool,
    ) -> RPCResult<ReturnSignature, (), Self::Error>;

    /// Verifies the signature of a message using the provided public key and signature.
    ///
    /// **Parameters**:  
    /// - `message` (`string`, required): The original message that was signed.  
    /// - `publicKey` (`Ed25519PublicKey`, required): The public key associated with the account that signed the message.  
    /// - `signature` (`Ed25519Signature`, required): The cryptographic signature to be verified, with the `R` and `s` components concatenated.  
    /// - `isHex` (`bool`, optional): Specifies whether the `message` is in hexadecimal format (`true`) or raw string format (`false`). Defaults to `false`.  
    ///
    /// **Returns**:  
    /// - `bool`:  
    ///   - `true` if the signature is valid for the given message and public key.  
    ///   - `false` if the signature is invalid.  
    ///
    /// **Notes**:  
    /// - If `isHex` is set to `true`, the `message` must be a valid hexadecimal string; otherwise, verification will fail.  
    ///
    /// **Example Response**:  
    /// ```json
    /// {
    ///   "data": true,
    ///   "metadata": null
    /// }
    /// ```
    async fn verify_signature(
        &mut self,
        message: String,
        public_key: Ed25519PublicKey,
        signature: Ed25519Signature,
        is_hex: bool,
    ) -> RPCResult<bool, (), Self::Error>;
}
