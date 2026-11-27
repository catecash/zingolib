//! Balance methods and types for `crate::wallet::LightWallet`.

use pepper_sync::wallet::{
    IronwoodNote, KeyIdInterface, NoteInterface, OrchardNote, OutputInterface, SaplingNote,
    TransactionRecord, TransparentCoin,
};
use zcash_client_backend::data_api::WalletRead;
use zcash_primitives::transaction::fees::zip317::MARGINAL_FEE;
use zcash_protocol::{PoolType, value::Zatoshis};

use crate::utils;

use super::{
    LightWallet,
    error::{BalanceError, KeyError},
    keys::unified::UnifiedKeyStore,
};

fn checked_sum_atomic(amounts: impl IntoIterator<Item = u128>) -> Result<u128, BalanceError> {
    amounts.into_iter().try_fold(0u128, |sum, amount| {
        sum.checked_add(amount).ok_or(BalanceError::Overflow)
    })
}

/// Balance for a wallet account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountBalance {
    /// Sum of unspent ironwood note values in confirmed blocks excluding dust.
    pub confirmed_ironwood_balance: Option<Zatoshis>,
    /// Sum of unspent ironwood note values in unconfirmed blocks excluding dust.
    pub unconfirmed_ironwood_balance: Option<Zatoshis>,
    /// Sum of confirmed and unconfirmed ironwood balances.
    pub total_ironwood_balance: Option<Zatoshis>,

    /// Sum of unspent orchard note values in confirmed blocks excluding dust.
    pub confirmed_orchard_balance: Option<Zatoshis>,
    /// Sum of unspent orchard note values in unconfirmed blocks excluding dust.
    pub unconfirmed_orchard_balance: Option<Zatoshis>,
    /// Sum of confirmed and unconfirmed orchard balances.
    pub total_orchard_balance: Option<Zatoshis>,

    /// Sum of unspent sapling note values in confirmed blocks excluding dust.
    pub confirmed_sapling_balance: Option<Zatoshis>,
    /// Sum of unspent sapling note values in unconfirmed blocks excluding dust.
    pub unconfirmed_sapling_balance: Option<Zatoshis>,
    /// Sum of confirmed and unconfirmed sapling balances.
    pub total_sapling_balance: Option<Zatoshis>,

    /// Sum of unspent transparent coin values in confirmed blocks excluding dust.
    pub confirmed_transparent_balance: Option<Zatoshis>,
    /// Sum of unspent transparent coin values in unconfirmed blocks excluding dust.
    pub unconfirmed_transparent_balance: Option<Zatoshis>,
    /// Sum of confirmed and unconfirmed transparent balances.
    pub total_transparent_balance: Option<Zatoshis>,
}

impl std::fmt::Display for AccountBalance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[
    confirmed_ironwood_balance: {}
    unconfirmed_ironwood_balance: {}
    total_ironwood_balance: {}

    confirmed_orchard_balance: {}
    unconfirmed_orchard_balance: {}
    total_orchard_balance: {}

    confirmed_sapling_balance: {}
    unconfirmed_sapling_balance: {}
    total_sapling_balance: {}

    confirmed_transparent_balance: {}
    unconfirmed_transparent_balance: {}
    total_transparent_balance: {}
]",
            self.confirmed_ironwood_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.unconfirmed_ironwood_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.total_ironwood_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.confirmed_orchard_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.unconfirmed_orchard_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.total_orchard_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.confirmed_sapling_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.unconfirmed_sapling_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.total_sapling_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.confirmed_transparent_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.unconfirmed_transparent_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
            self.total_transparent_balance
                .map_or("no view capability".to_string(), |zats| {
                    format_zatoshis(zats)
                }),
        )
    }
}

impl From<AccountBalance> for json::JsonValue {
    fn from(value: AccountBalance) -> Self {
        let amount = |balance: Option<Zatoshis>| balance.map(Zatoshis::into_u64);
        json::object! {
            "confirmed_ironwood_balance" => amount(value.confirmed_ironwood_balance),
            "unconfirmed_ironwood_balance" => amount(value.unconfirmed_ironwood_balance),
            "total_ironwood_balance" => amount(value.total_ironwood_balance),
            "confirmed_orchard_balance" => amount(value.confirmed_orchard_balance),
            "unconfirmed_orchard_balance" => amount(value.unconfirmed_orchard_balance),
            "total_orchard_balance" => amount(value.total_orchard_balance),
            "confirmed_sapling_balance" => amount(value.confirmed_sapling_balance),
            "unconfirmed_sapling_balance" => amount(value.unconfirmed_sapling_balance),
            "total_sapling_balance" => amount(value.total_sapling_balance),
            "confirmed_transparent_balance" => amount(value.confirmed_transparent_balance),
            "unconfirmed_transparent_balance" => amount(value.unconfirmed_transparent_balance),
            "total_transparent_balance" => amount(value.total_transparent_balance),
        }
    }
}

fn format_zatoshis(zatoshis: Zatoshis) -> String {
    let zats = zatoshis.into_u64().to_string();
    let digits: Vec<char> = zats.chars().rev().collect();
    let mut formatted = String::new();

    for (index, digit) in digits.iter().enumerate() {
        if index > 0 && index % 3 == 0 {
            formatted.push('_');
        }
        formatted.push(*digit);
    }

    formatted.chars().rev().collect()
}

impl LightWallet {
    /// Serializes Catecash account balances with checked 128-bit pool sums.
    pub fn catecash_balance_json(
        &self,
        account_id: zip32::AccountId,
    ) -> Result<json::JsonValue, BalanceError> {
        let ironwood = self.wide_pool_balance::<IronwoodNote>(account_id)?;
        let orchard = self.wide_pool_balance::<OrchardNote>(account_id)?;
        let sapling = self.wide_pool_balance::<SaplingNote>(account_id)?;
        let transparent = self.wide_pool_balance::<TransparentCoin>(account_id)?;
        let amount = |balance: Option<u128>| balance.map(|atomic| atomic.to_string());
        Ok(json::object! {
            "confirmed_ironwood_balance" => amount(ironwood.0),
            "unconfirmed_ironwood_balance" => amount(ironwood.1),
            "total_ironwood_balance" => amount(ironwood.2),
            "confirmed_orchard_balance" => amount(orchard.0),
            "unconfirmed_orchard_balance" => amount(orchard.1),
            "total_orchard_balance" => amount(orchard.2),
            "confirmed_sapling_balance" => amount(sapling.0),
            "unconfirmed_sapling_balance" => amount(sapling.1),
            "total_sapling_balance" => amount(sapling.2),
            "confirmed_transparent_balance" => amount(transparent.0),
            "unconfirmed_transparent_balance" => amount(transparent.1),
            "total_transparent_balance" => amount(transparent.2),
        })
    }

    fn wide_pool_balance<Op: OutputInterface>(
        &self,
        account_id: zip32::AccountId,
    ) -> Result<(Option<u128>, Option<u128>, Option<u128>), BalanceError> {
        let confirmed = self.get_filtered_balance_wide::<Op, _>(
            |output, transaction| {
                Op::value(output) > MARGINAL_FEE.into_u64()
                    && transaction.status().is_confirmed()
                    && self.is_transparent_output_mature::<Op>(transaction)
            },
            account_id,
        );
        let unconfirmed = self.get_filtered_balance_wide::<Op, _>(
            |output, transaction| {
                Op::value(output) > MARGINAL_FEE.into_u64() && transaction.status().is_pending()
            },
            account_id,
        );
        match (confirmed, unconfirmed) {
            (
                Err(BalanceError::KeyError(KeyError::NoViewCapability)),
                Err(BalanceError::KeyError(KeyError::NoViewCapability)),
            ) => Ok((None, None, None)),
            (Ok(confirmed), Ok(unconfirmed)) => Ok((
                Some(confirmed),
                Some(unconfirmed),
                Some(
                    confirmed
                        .checked_add(unconfirmed)
                        .ok_or(BalanceError::Overflow)?,
                ),
            )),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }
    /// Checks a transparent coinbase output against the wallet chain's maturity rule.
    fn is_transparent_output_mature<Op: OutputInterface>(
        &self,
        transaction: &TransactionRecord,
    ) -> bool {
        // Only transparent outputs need coinbase maturity check
        if Op::POOL_TYPE != PoolType::Transparent {
            return true;
        }

        // Check if this is a coinbase transaction
        let is_coinbase = transaction
            .transaction()
            .transparent_bundle()
            .is_some_and(|bundle| bundle.is_coinbase());

        if is_coinbase {
            let current_height = self
                .sync_state
                .last_known_chain_height()
                .unwrap_or(self.birthday);
            let tx_height = transaction.status().get_height();

            // Work with u32 values
            let current_height_u32: u32 = current_height.into();
            let tx_height_u32: u32 = tx_height.into();

            if current_height_u32 < tx_height_u32 {
                return false;
            }

            let confirmations = current_height_u32 - tx_height_u32;
            confirmations >= self.coinbase_maturity_blocks()
        } else {
            true
        }
    }

    /// Returns account balance.
    pub fn account_balance(
        &self,
        account_id: zip32::AccountId,
    ) -> Result<AccountBalance, BalanceError> {
        let confirmed_ironwood_balance =
            match self.confirmed_balance_excluding_dust::<IronwoodNote>(account_id) {
                Ok(zats) => Some(zats),
                Err(BalanceError::KeyError(KeyError::NoViewCapability)) => None,
                Err(e) => return Err(e),
            };
        let unconfirmed_ironwood_balance =
            match self.unconfirmed_balance_excluding_dust::<IronwoodNote>(account_id) {
                Ok(zats) => Some(zats),
                Err(BalanceError::KeyError(KeyError::NoViewCapability)) => None,
                Err(e) => return Err(e),
            };
        let total_ironwood_balance = confirmed_ironwood_balance
            .and_then(|confirmed| unconfirmed_ironwood_balance + confirmed);

        let confirmed_orchard_balance =
            match self.confirmed_balance_excluding_dust::<OrchardNote>(account_id) {
                Ok(zats) => Some(zats),
                Err(BalanceError::KeyError(KeyError::NoViewCapability)) => None,
                Err(e) => return Err(e),
            };
        let unconfirmed_orchard_balance =
            match self.unconfirmed_balance_excluding_dust::<OrchardNote>(account_id) {
                Ok(zats) => Some(zats),
                Err(BalanceError::KeyError(KeyError::NoViewCapability)) => None,
                Err(e) => return Err(e),
            };
        let total_orchard_balance =
            confirmed_orchard_balance.and_then(|confirmed| unconfirmed_orchard_balance + confirmed);

        let confirmed_sapling_balance =
            match self.confirmed_balance_excluding_dust::<SaplingNote>(account_id) {
                Ok(zats) => Some(zats),
                Err(BalanceError::KeyError(KeyError::NoViewCapability)) => None,
                Err(e) => return Err(e),
            };
        let unconfirmed_sapling_balance =
            match self.unconfirmed_balance_excluding_dust::<SaplingNote>(account_id) {
                Ok(zats) => Some(zats),
                Err(BalanceError::KeyError(KeyError::NoViewCapability)) => None,
                Err(e) => return Err(e),
            };
        let total_sapling_balance =
            confirmed_sapling_balance.and_then(|confirmed| unconfirmed_sapling_balance + confirmed);

        let confirmed_transparent_balance =
            match self.confirmed_balance_excluding_dust::<TransparentCoin>(account_id) {
                Ok(zats) => Some(zats),
                Err(BalanceError::KeyError(KeyError::NoViewCapability)) => None,
                Err(e) => return Err(e),
            };
        let unconfirmed_transparent_balance =
            match self.unconfirmed_balance_excluding_dust::<TransparentCoin>(account_id) {
                Ok(zats) => Some(zats),
                Err(BalanceError::KeyError(KeyError::NoViewCapability)) => None,
                Err(e) => return Err(e),
            };
        let total_transparent_balance = confirmed_transparent_balance
            .and_then(|confirmed| unconfirmed_transparent_balance + confirmed);

        Ok(AccountBalance {
            confirmed_ironwood_balance,
            unconfirmed_ironwood_balance,
            total_ironwood_balance,
            confirmed_orchard_balance,
            unconfirmed_orchard_balance,
            total_orchard_balance,
            confirmed_sapling_balance,
            unconfirmed_sapling_balance,
            total_sapling_balance,
            confirmed_transparent_balance,
            unconfirmed_transparent_balance,
            total_transparent_balance,
        })
    }

    /// Returns the total balance of the all unspent outputs of a given pool type and `account_id` matching the output
    /// and transaction criteria specified by the `filter_function`.
    ///
    /// # Error
    ///
    /// Returns an error if:
    /// - no keys are found for the given `account_id`
    /// - the UFVK does not have viewing capability for the given pool type
    /// - the balance summation exceeds the valid range of zatoshis
    pub fn get_filtered_balance<Op, F>(
        &self,
        filter_function: F,
        account_id: zip32::AccountId,
    ) -> Result<Zatoshis, BalanceError>
    where
        Op: OutputInterface,
        F: Fn(&Op, &TransactionRecord) -> bool,
    {
        let balance = self.get_filtered_balance_wide::<Op, F>(filter_function, account_id)?;
        let narrow = u64::try_from(balance).map_err(|_| BalanceError::Overflow)?;
        Ok(utils::conversion::zatoshis_from_u64(narrow)?)
    }

    fn get_filtered_balance_wide<Op, F>(
        &self,
        filter_function: F,
        account_id: zip32::AccountId,
    ) -> Result<u128, BalanceError>
    where
        Op: OutputInterface,
        F: Fn(&Op, &TransactionRecord) -> bool,
    {
        let pending_spends = pepper_sync::pending_spends(&self.wallet_transactions);
        match &self
            .unified_key_store
            .get(&account_id)
            .ok_or(KeyError::NoAccountKeys)?
        {
            UnifiedKeyStore::Spend(_) => (),
            UnifiedKeyStore::View(ufvk) => match Op::POOL_TYPE {
                PoolType::Transparent => {
                    if ufvk.p2pkh().is_none() {
                        return Err(KeyError::NoViewCapability.into());
                    }
                }
                PoolType::SAPLING => {
                    if ufvk.sapling().is_none() {
                        return Err(KeyError::NoViewCapability.into());
                    }
                }
                PoolType::ORCHARD => {
                    if ufvk.orchard().is_none() {
                        return Err(KeyError::NoViewCapability.into());
                    }
                }
                PoolType::IRONWOOD => {
                    // Ironwood reuses the Orchard keys: viewing capability
                    // for the pool is the Orchard FVK.
                    if ufvk.orchard().is_none() {
                        return Err(KeyError::NoViewCapability.into());
                    }
                }
            },
            UnifiedKeyStore::Empty => return Err(KeyError::NoViewCapability.into()),
        }

        self.wallet_transactions
            .values()
            .try_fold(0u128, |acc, transaction| {
                let transaction_balance = checked_sum_atomic(
                    Op::transaction_outputs(transaction)
                        .iter()
                        .filter(|&output| {
                            filter_function(output, transaction)
                                && output.spending_transaction().is_none()
                                && !pending_spends.contains_key(&output.output_id())
                                && output.key_id().account_id() == account_id
                        })
                        .map(|output| u128::from(Op::value(output))),
                )?;
                acc.checked_add(transaction_balance)
                    .ok_or(BalanceError::Overflow)
            })
    }

    /// Returns total balance of unspent notes in confirmed blocks for a given shielded pool and `account_id`.
    ///
    /// # Error
    ///
    /// Returns an error if:
    /// - no keys are found for the given `account_id`
    /// - the UFVK does not have viewing capability for the given pool type
    /// - the balance summation exceeds the valid range of zatoshis
    pub fn confirmed_balance<Op>(
        &self,
        account_id: zip32::AccountId,
    ) -> Result<Zatoshis, BalanceError>
    where
        Op: OutputInterface,
    {
        self.get_filtered_balance::<Op, _>(
            |_output, transaction: &TransactionRecord| {
                transaction.status().is_confirmed()
                    && self.is_transparent_output_mature::<Op>(transaction)
            },
            account_id,
        )
    }

    /// Returns total balance of unspent notes in confirmed blocks for a given shielded pool and `account_id`,
    /// excluding any notes with value less than marginal fee (`5_000`).
    ///
    /// # Error
    ///
    /// Returns an error if:
    /// - no keys are found for the given `account_id`
    /// - the UFVK does not have viewing capability for the given pool type
    /// - the balance summation exceeds the valid range of zatoshis
    pub fn confirmed_balance_excluding_dust<Op>(
        &self,
        account_id: zip32::AccountId,
    ) -> Result<Zatoshis, BalanceError>
    where
        Op: OutputInterface,
    {
        self.get_filtered_balance::<Op, _>(
            |output, transaction: &TransactionRecord| {
                Op::value(output) > MARGINAL_FEE.into_u64()
                    && transaction.status().is_confirmed()
                    && self.is_transparent_output_mature::<Op>(transaction)
            },
            account_id,
        )
    }

    /// Returns total balance of unspent notes not yet confirmed on the block chain for a given shielded pool and
    /// `account_id`.
    ///
    /// # Error
    ///
    /// Returns an error if:
    /// - no keys are found for the given `account_id`
    /// - the UFVK does not have viewing capability for the given pool type
    /// - the balance summation exceeds the valid range of zatoshis
    pub fn unconfirmed_balance<Op>(
        &self,
        account_id: zip32::AccountId,
    ) -> Result<Zatoshis, BalanceError>
    where
        Op: OutputInterface,
    {
        self.get_filtered_balance::<Op, _>(
            |_, transaction: &TransactionRecord| transaction.status().is_pending(),
            account_id,
        )
    }

    /// Returns total balance of unspent notes not yet confirmed on the block chain for a given shielded pool and
    /// `account_id`, excluding any notes with value less than marginal fee (`5_000`).
    ///
    /// # Error
    ///
    /// Returns an error if:
    /// - no keys are found for the given `account_id`
    /// - the UFVK does not have viewing capability for the given pool type
    /// - the balance summation exceeds the valid range of zatoshis
    pub fn unconfirmed_balance_excluding_dust<Op>(
        &self,
        account_id: zip32::AccountId,
    ) -> Result<Zatoshis, BalanceError>
    where
        Op: OutputInterface,
    {
        self.get_filtered_balance::<Op, _>(
            |note, transaction: &TransactionRecord| {
                Op::value(note) > MARGINAL_FEE.into_u64() && transaction.status().is_pending()
            },
            account_id,
        )
    }

    /// Returns total balance of spendable notes for a given shielded pool and `account_id`.
    ///
    /// Spendable notes are:
    /// - confirmed
    /// - not dust (note value larger than `5_000` zats)
    /// - the wallet can build a witness for the note's commitment
    /// - satisfy the number of minimum confirmations set by the wallet
    /// - the nullifier derived from the note does not appear in a transaction input (spend) on chain
    ///
    /// If `include_potentially_spent_notes` is `true`, notes will be included even if the wallet's current sync state
    /// is incomplete and it is unknown if the note has already been spent (the nullifier has not appeared on chain *yet*).
    ///
    /// # Error
    ///
    /// Returns an error if:
    /// - no keys are found for the given `account_id`
    /// - the UFVK does not have viewing capability for the given pool type
    /// - the balance summation exceeds the valid range of zatoshis
    pub fn spendable_balance<N>(
        &self,
        account_id: zip32::AccountId,
        include_potentially_spent_notes: bool,
    ) -> Result<Zatoshis, BalanceError>
    where
        N: NoteInterface,
    {
        let Some(spend_evidence_floor) = self.spend_evidence_floor(false) else {
            return Ok(Zatoshis::ZERO);
        };
        let Some((_, anchor_height)) = self
            .get_target_and_anchor_heights(self.wallet_settings.min_confirmations)
            .expect("infallible")
        else {
            return Ok(Zatoshis::ZERO);
        };

        let spendable_balance = self.get_filtered_balance::<N, _>(
            |note, transaction: &TransactionRecord| {
                N::value(note) > MARGINAL_FEE.into_u64()
                    && transaction.status().is_confirmed()
                    && transaction.status().get_height() <= anchor_height
                    && note.position().is_some()
                    && self.can_build_witness::<N>(transaction.status().get_height(), anchor_height)
                    && (include_potentially_spent_notes
                        || self.note_spends_confirmed(
                            transaction.status().get_height(),
                            spend_evidence_floor,
                            note.refetch_nullifier_ranges(),
                        ))
            },
            account_id,
        )?;

        Ok(spendable_balance)
    }

    /// Returns total spendable balance of all shielded pools of a given `account_id`.
    ///
    /// See [`Self::spendable_balance`] for more information.
    ///
    /// # Error
    ///
    /// Returns an error if:
    /// - no keys are found for the given `account_id`
    /// - the balance summation exceeds the valid range of zatoshis
    pub fn shielded_spendable_balance(
        &self,
        account_id: zip32::AccountId,
        include_potentially_spent_notes: bool,
    ) -> Result<Zatoshis, BalanceError> {
        let ironwood_balance = match self
            .spendable_balance::<IronwoodNote>(account_id, include_potentially_spent_notes)
        {
            Ok(zats) => Ok(zats),
            Err(BalanceError::KeyError(KeyError::NoViewCapability)) => Ok(Zatoshis::ZERO),
            Err(e) => Err(e),
        }?;
        let orchard_balance = match self
            .spendable_balance::<OrchardNote>(account_id, include_potentially_spent_notes)
        {
            Ok(zats) => Ok(zats),
            Err(BalanceError::KeyError(KeyError::NoViewCapability)) => Ok(Zatoshis::ZERO),
            Err(e) => Err(e),
        }?;
        let sapling_balance = match self
            .spendable_balance::<SaplingNote>(account_id, include_potentially_spent_notes)
        {
            Ok(zats) => Ok(zats),
            Err(BalanceError::KeyError(KeyError::NoViewCapability)) => Ok(Zatoshis::ZERO),
            Err(e) => Err(e),
        }?;

        (orchard_balance + sapling_balance)
            .and_then(|balance| balance + ironwood_balance)
            .ok_or(BalanceError::Overflow)
    }
}

#[cfg(any(test, feature = "testutils"))]
mod test {
    #[test]
    fn tests_that_catecash_aggregate_crosses_legacy_money_cap() {
        let balance = super::checked_sum_atomic([2_100_000_000_000_000; 5]);
        assert_eq!(balance.ok(), Some(10_500_000_000_000_000));
    }

    #[test]
    fn tests_that_catecash_aggregate_rejects_u128_overflow() {
        let balance = super::checked_sum_atomic([u128::MAX, 1]);
        assert!(matches!(balance, Err(super::BalanceError::Overflow)));
    }

    // FIXME: zingo2 rewrite as an integration test
    // #[tokio::test]
    // async fn confirmed_balance_excluding_dust() {
    //     let wallet = LightWallet::new(
    //         ZingoConfigBuilder::default().create().chain,
    //         WalletBase::FreshEntropy,
    //         1.into(),
    //     )
    //     .unwrap();
    //     let confirmed_tx_record = TransactionRecordBuilder::default()
    //         .status(ConfirmationStatus::Confirmed(80.into()))
    //         .transparent_outputs(TransparentOutputBuilder::default())
    //         .sapling_notes(SaplingNoteBuilder::default())
    //         .sapling_notes(SaplingNoteBuilder::default())
    //         .sapling_notes(
    //             SaplingNoteBuilder::default()
    //                 .note(
    //                     SaplingCryptoNoteBuilder::default()
    //                         .value(sapling_crypto::value::NoteValue::from_raw(3_000))
    //                         .clone(),
    //                 )
    //                 .clone(),
    //         )
    //         .orchard_notes(OrchardNoteBuilder::default())
    //         .orchard_notes(OrchardNoteBuilder::default())
    //         .orchard_notes(
    //             OrchardNoteBuilder::default()
    //                 .note(
    //                     OrchardCryptoNoteBuilder::default()
    //                         .value(orchard::value::NoteValue::from_raw(5_001))
    //                         .clone(),
    //                 )
    //                 .clone(),
    //         )
    //         .orchard_notes(
    //             OrchardNoteBuilder::default()
    //                 .note(
    //                     OrchardCryptoNoteBuilder::default()
    //                         .value(orchard::value::NoteValue::from_raw(2_000))
    //                         .clone(),
    //                 )
    //                 .clone(),
    //         )
    //         .build();
    //     let mempool_tx_record = TransactionRecordBuilder::default()
    //         .status(ConfirmationStatus::Mempool(95.into()))
    //         .transparent_outputs(TransparentOutputBuilder::default())
    //         .sapling_notes(SaplingNoteBuilder::default())
    //         .orchard_notes(OrchardNoteBuilder::default())
    //         .build();
    //     {
    //         let mut tx_map = wallet
    //             .transaction_context
    //             .transaction_metadata_set
    //             .write()
    //             .await;
    //         tx_map
    //             .transaction_records_by_id
    //             .insert_transaction_record(confirmed_tx_record);
    //         tx_map
    //             .transaction_records_by_id
    //             .insert_transaction_record(mempool_tx_record);
    //     }

    //     assert_eq!(
    //         wallet.confirmed_balance_excluding_dust::<Sapling>().await,
    //         Some(400_000)
    //     );
    //     assert_eq!(
    //         wallet.confirmed_balance_excluding_dust::<Orchard>().await,
    //         Some(1_605_001)
    //     );
    // }
}
