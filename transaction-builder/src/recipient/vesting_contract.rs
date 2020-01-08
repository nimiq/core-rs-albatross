use failure::Fail;

use keys::Address;
use primitives::coin::Coin;
use transaction::account::vesting_contract::CreationTransactionData as VestingCreationData;

use crate::recipient::Recipient;
use std::ops::Div;

/// Building a vesting recipient can fail if mandatory fields are not set.
/// In these cases, a `VestingRecipientBuilderError` is returned.
#[derive(Debug, Fail)]
pub enum VestingRecipientBuilderError {
    /// The `owner` field of the [`VestingRecipientBuilder`] has not been set.
    /// Call [`with_owner`] to set this field.
    ///
    /// [`VestingRecipientBuilder`]: struct.VestingRecipientBuilder.html
    /// [`with_owner`]: struct.VestingRecipientBuilder.html#method.with_owner
    #[fail(display = "The vesting owner address is missing.")]
    NoOwner,
    /// The `start` field of the [`VestingRecipientBuilder`] has not been set.
    /// Call [`with_start_block`] to set this field.
    ///
    /// [`VestingRecipientBuilder`]: struct.VestingRecipientBuilder.html
    /// [`with_start_block`]: struct.VestingRecipientBuilder.html#method.with_start_block
    #[fail(display = "The vesting start block is missing.")]
    NoStartBlock,
    /// The `step_blocks` field of the [`VestingRecipientBuilder`] has not been set.
    /// Call [`with_step_distance`] to set this field.
    ///
    /// [`VestingRecipientBuilder`]: struct.VestingRecipientBuilder.html
    /// [`with_step_distance`]: struct.VestingRecipientBuilder.html#method.with_step_distance
    #[fail(display = "The vesting step distance is missing.")]
    NoStepDistance,
    /// The `step_amount` field of the [`VestingRecipientBuilder`] has not been set.
    /// Call [`with_step_amount`] to set this field.
    ///
    /// [`VestingRecipientBuilder`]: struct.VestingRecipientBuilder.html
    /// [`with_step_amount`]: struct.VestingRecipientBuilder.html#method.with_step_amount
    #[fail(display = "The vesting step amount is missing.")]
    NoStepAmount,
    /// The `total_amount` field of the [`VestingRecipientBuilder`] has not been set.
    /// Call [`with_total_amount`] to set this field.
    ///
    /// [`VestingRecipientBuilder`]: struct.VestingRecipientBuilder.html
    /// [`with_total_amount`]: struct.VestingRecipientBuilder.html#method.with_total_amount
    #[fail(display = "The vesting total amount is missing.")]
    NoTotalAmount,
}

/// A `VestingRecipientBuilder` can be used to create new vesting contracts.
/// A vesting contract locks funds of a single `owner`.
/// The owner does not necessarily needs to coincide with the transaction's sender.
///
/// The total of `total_amount` of funds can be unlocked over time:
/// The contracts functionality begins at a starting block `start_block`.
/// Starting from there, `step_amount` funds will be released every `step_distance` blocks.
///
/// That means that the first funds can be withdrawn at blockchain height
/// `start_block + step_distance`.
#[derive(Default)]
pub struct VestingRecipientBuilder {
    owner: Option<Address>,
    start_block: Option<u32>,
    step_distance: Option<u32>,
    step_amount: Option<Coin>,
    total_amount: Option<Coin>,
}

impl VestingRecipientBuilder {
    /// Creates a new vesting contract with funds owned by `owner`.
    pub fn new(owner: Address) -> Self {
        let mut builder: VestingRecipientBuilder = Default::default();
        builder.with_owner(owner);
        builder
    }

    /// Creates a simple vesting contract that will release `amount` coins to `owner`
    /// at blockchain height `start_block`.
    pub fn new_single_step(owner: Address, start_block: u32, amount: Coin) -> Self {
        let mut builder = Self::new(owner);
        builder.with_start_block(0)
            .with_step_distance(start_block)
            .with_total_amount(amount)
            .with_step_amount(amount);
        builder
    }

    /// Sets the `owner` of the funds in the contract.
    pub fn with_owner(&mut self, owner: Address) -> &mut Self {
        self.owner = Some(owner);
        self
    }

    /// Sets the `start_block` for the release schedule.
    /// The first `step_amount` funds will be released at time `start_block + step_distance`.
    pub fn with_start_block(&mut self, start_block: u32) -> &mut Self {
        self.start_block = Some(start_block);
        self
    }

    /// Sets the `total_amount` for the release schedule.
    /// If this amount is less than the transaction value, the remaining funds are available
    /// with immediate effect.
    pub fn with_total_amount(&mut self, amount: Coin) -> &mut Self {
        self.total_amount = Some(amount);
        self
    }

    /// This convenience function allows to quickly create a release schedule of `num_steps`
    /// payouts starting at `start_block + step_distance`.
    pub fn with_steps(&mut self, total_amount: Coin, start_block: u32, step_distance: u32, num_steps: u32) -> &mut Self {
        let step_amount = total_amount.div(u64::from(num_steps));
        self.with_total_amount(total_amount)
            .with_start_block(start_block)
            .with_step_distance(step_distance)
            .with_step_amount(step_amount);
        self
    }

    /// Sets the distance between releasing funds from the contract.
    pub fn with_step_distance(&mut self, step_blocks: u32) -> &mut Self {
        self.step_distance = Some(step_blocks);
        self
    }

    /// Sets the amount of funds to become available with each release.
    pub fn with_step_amount(&mut self, step_amount: Coin) -> &mut Self {
        self.step_amount = Some(step_amount);
        self
    }

    /// This method tries putting together the contract creation,
    /// returning a [`Recipient`] in case of success.
    /// In case of a failure, it returns a [`VestingRecipientBuilderError`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::Recipient;
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    ///
    /// let owner = Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap();
    /// let mut recipient_builder = Recipient::new_vesting_builder(owner);
    /// recipient_builder.with_steps(
    ///     Coin::from_u64_unchecked(10_000), // total amount
    ///     13377, // start block
    ///     100, // every 100 blocks
    ///     5 // five steps
    /// );
    /// let recipient = recipient_builder.generate();
    /// assert!(recipient.is_ok());
    /// ```
    ///
    /// [`Recipient`]: ../enum.Recipient.html
    /// [`VestingRecipientBuilderError`]: enum.VestingRecipientBuilderError.html
    pub fn generate(self) -> Result<Recipient, VestingRecipientBuilderError> {
        Ok(Recipient::VestingCreation {
            data: VestingCreationData {
                owner: self.owner.ok_or(VestingRecipientBuilderError::NoOwner)?,
                start: self.start_block.ok_or(VestingRecipientBuilderError::NoStartBlock)?,
                step_blocks: self.step_distance.ok_or(VestingRecipientBuilderError::NoStepDistance)?,
                step_amount: self.step_amount.ok_or(VestingRecipientBuilderError::NoStepAmount)?,
                total_amount: self.total_amount.ok_or(VestingRecipientBuilderError::NoTotalAmount)?
            }
        })
    }
}
