use std::ops::Div;

use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::vesting_contract::CreationTransactionData as VestingCreationData;
use thiserror::Error;

use crate::recipient::Recipient;

/// Building a vesting recipient can fail if mandatory fields are not set.
/// In these cases, a `VestingRecipientBuilderError` is returned.
#[derive(Debug, Error)]
pub enum VestingRecipientBuilderError {
    /// The `owner` field of the [`VestingRecipientBuilder`] has not been set.
    /// Call [`with_owner`] to set this field.
    ///
    /// [`VestingRecipientBuilder`]: struct.VestingRecipientBuilder.html
    /// [`with_owner`]: struct.VestingRecipientBuilder.html#method.with_owner
    #[error("The vesting owner address is missing.")]
    NoOwner,
    /// The `start` field of the [`VestingRecipientBuilder`] has not been set.
    /// Call [`with_start_block`] to set this field.
    ///
    /// [`VestingRecipientBuilder`]: struct.VestingRecipientBuilder.html
    /// [`with_start_block`]: struct.VestingRecipientBuilder.html#method.with_start_block
    #[error("The vesting start block is missing.")]
    NoStartBlock,
    /// The `step_blocks` field of the [`VestingRecipientBuilder`] has not been set.
    /// Call [`with_time_step`] to set this field.
    ///
    /// [`VestingRecipientBuilder`]: struct.VestingRecipientBuilder.html
    /// [`with_time_step`]: struct.VestingRecipientBuilder.html#method.with_time_step
    #[error("The vesting time step is missing.")]
    NoTimeStep,
    /// The `step_amount` field of the [`VestingRecipientBuilder`] has not been set.
    /// Call [`with_step_amount`] to set this field.
    ///
    /// [`VestingRecipientBuilder`]: struct.VestingRecipientBuilder.html
    /// [`with_step_amount`]: struct.VestingRecipientBuilder.html#method.with_step_amount
    #[error("The vesting step amount is missing.")]
    NoStepAmount,
    /// The `total_amount` field of the [`VestingRecipientBuilder`] has not been set.
    /// Call [`with_total_amount`] to set this field.
    ///
    /// [`VestingRecipientBuilder`]: struct.VestingRecipientBuilder.html
    /// [`with_total_amount`]: struct.VestingRecipientBuilder.html#method.with_total_amount
    #[error("The vesting total amount is missing.")]
    NoTotalAmount,
}

/// A `VestingRecipientBuilder` can be used to create new vesting contracts.
/// A vesting contract locks funds of a single `owner`.
/// The owner does not necessarily needs to coincide with the transaction's sender.
///
/// The total of `total_amount` of funds can be unlocked over time:
/// The contracts functionality begins at a starting block `start_block`.
/// Starting from there, `step_amount` funds will be released every `time_step` blocks.
///
/// That means that the first funds can be withdrawn at blockchain height
/// `start_block + time_step`.
#[derive(Default)]
pub struct VestingRecipientBuilder {
    owner: Option<Address>,
    start_time: Option<u64>,
    time_step: Option<u64>,
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
    /// at blockchain height `start_time`.
    pub fn new_single_step(owner: Address, start_time: u64, amount: Coin) -> Self {
        let mut builder = Self::new(owner);
        builder
            .with_start_time(0)
            .with_time_step(start_time)
            .with_total_amount(amount)
            .with_step_amount(amount);
        builder
    }

    /// Sets the `owner` of the funds in the contract.
    pub fn with_owner(&mut self, owner: Address) -> &mut Self {
        self.owner = Some(owner);
        self
    }

    /// Sets the `start_time` for the release schedule.
    /// The first `time_step` funds will be released at time `start_time + time_step`.
    pub fn with_start_time(&mut self, start_time: u64) -> &mut Self {
        self.start_time = Some(start_time);
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
    /// payouts starting at `start_time + time_step`.
    pub fn with_steps(
        &mut self,
        total_amount: Coin,
        start_time: u64,
        time_step: u64,
        num_steps: u32,
    ) -> &mut Self {
        let step_amount = total_amount.div(u64::from(num_steps));
        self.with_total_amount(total_amount)
            .with_start_time(start_time)
            .with_time_step(time_step)
            .with_step_amount(step_amount);
        self
    }

    /// Sets the distance between releasing funds from the contract.
    pub fn with_time_step(&mut self, time_step: u64) -> &mut Self {
        self.time_step = Some(time_step);
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
                start_time: self
                    .start_time
                    .ok_or(VestingRecipientBuilderError::NoStartBlock)?,
                time_step: self
                    .time_step
                    .ok_or(VestingRecipientBuilderError::NoTimeStep)?,
                step_amount: self
                    .step_amount
                    .ok_or(VestingRecipientBuilderError::NoStepAmount)?
                    .into(),
                total_amount: self
                    .total_amount
                    .ok_or(VestingRecipientBuilderError::NoTotalAmount)?
                    .into(),
            },
        })
    }
}
