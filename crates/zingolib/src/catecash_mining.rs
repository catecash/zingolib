use http::Uri;
use secp256k1::{PublicKey, SecretKey};
use thiserror::Error;
use zcash_address::{ToAddress as _, ZcashAddress};
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding};
use zcash_primitives::transaction::fees::{FeeRule as _, transparent::InputSize, zip317};
use zcash_protocol::consensus::Parameters;
use zcash_protocol::{consensus::BlockHeight, memo::MemoBytes, value::Zatoshis};
use zcash_transparent::address::TransparentAddress;
use zcash_transparent::builder::TransparentSigningSet;
use zcash_transparent::bundle::{OutPoint, TxOut};
use zl_netutils::lightwallet_protocol::{AddressList, GetAddressUtxosArg, GetAddressUtxosReply};
use zl_netutils::{Indexer as _, TransparentIndexer as _};

use crate::config::ChainType;
use crate::lightclient::{DEFAULT_REQUEST_TIMEOUT, verify_catecash_indexer};

const WIF_LENGTH: usize = 34;
const COMPRESSED_PUBLIC_KEY_MARKER: u8 = 1;
pub const COINBASE_MATURITY_BLOCKS: u64 = 240;
const IRONWOOD_ACTIONS_FOR_ONE_OUTPUT: usize = 2;

pub fn ironwood_sweep_fee(target_height: u32) -> Result<u64, MiningKeyError> {
    ironwood_sweep_fee_for_chain(ChainType::CatecashDevnet, target_height)
}

pub fn ironwood_sweep_fee_for_chain(
    chain: ChainType,
    target_height: u32,
) -> Result<u64, MiningKeyError> {
    zip317::FeeRule::standard()
        .fee_required(
            &chain,
            BlockHeight::from_u32(target_height),
            [InputSize::STANDARD_P2PKH],
            std::iter::empty::<usize>(),
            0,
            0,
            0,
            IRONWOOD_ACTIONS_FOR_ONE_OUTPUT,
        )
        .map(|fee| fee.into_u64())
        .map_err(|error| MiningKeyError::Sweep(error.to_string()))
}

#[derive(Debug, Error)]
pub enum MiningKeyError {
    #[error("invalid Catecash WIF checksum or encoding: {0}")]
    Wif(#[from] bs58::decode::Error),
    #[error("WIF has the wrong network, key length, or compression marker")]
    Format,
    #[error("WIF contains an invalid secp256k1 secret: {0}")]
    Secret(#[from] secp256k1::Error),
    #[error("Catecash indexer connection failed: {0}")]
    Connect(#[from] zl_netutils::GetClientError),
    #[error("Catecash indexer request failed: {0}")]
    Indexer(#[from] zl_netutils::Status),
    #[error("Catecash indexer returned an invalid UTXO: {0}")]
    Utxo(&'static str),
    #[error("mining reward is immature until block {0}")]
    Immature(u64),
    #[error("cannot build mining reward sweep: {0}")]
    Sweep(String),
}

pub struct MiningKey {
    secret: SecretKey,
    address: String,
    chain: ChainType,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MiningBalance {
    pub confirmed_zat: u128,
    pub spendable_next_block_zat: u128,
    pub next_spend_height: Option<u64>,
    pub tip_height: u64,
}

impl MiningKey {
    pub fn import_wif(wif: &str) -> Result<Self, MiningKeyError> {
        Self::import_wif_for_chain(wif, ChainType::CatecashDevnet)
    }

    pub fn import_wif_for_chain(wif: &str, chain: ChainType) -> Result<Self, MiningKeyError> {
        if !matches!(
            chain,
            ChainType::CatecashDevnet | ChainType::CatecashTestnet | ChainType::CatecashMainnet
        ) {
            return Err(MiningKeyError::Format);
        }
        let bytes = bs58::decode(wif).with_check(None).into_vec()?;
        let prefix = zcash_keys::constants::b58_secret_key_prefix(chain.network_type());
        if bytes.len() != WIF_LENGTH
            || bytes[0] != prefix[0]
            || bytes[WIF_LENGTH - 1] != COMPRESSED_PUBLIC_KEY_MARKER
        {
            return Err(MiningKeyError::Format);
        }
        let secret_bytes: [u8; 32] = bytes[1..WIF_LENGTH - 1]
            .try_into()
            .map_err(|_| MiningKeyError::Format)?;
        let secret = SecretKey::from_secret_bytes(secret_bytes)?;
        let public = PublicKey::from_secret_key(&secret);
        let TransparentAddress::PublicKeyHash(hash) = TransparentAddress::from_pubkey(&public)
        else {
            unreachable!("a public key derives a P2PKH address")
        };
        let address = ZcashAddress::from_transparent_p2pkh(chain.network_type(), hash).to_string();
        Ok(Self {
            secret,
            address,
            chain,
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn build_ironwood_sweep(
        &self,
        utxo: &GetAddressUtxosReply,
        target_height: u32,
        recipient: orchard::Address,
        anchor: orchard::Anchor,
    ) -> Result<Vec<u8>, MiningKeyError> {
        if utxo.address != self.address {
            return Err(MiningKeyError::Utxo("output belongs to another address"));
        }
        let public = PublicKey::from_secret_key(&self.secret);
        let address = TransparentAddress::from_pubkey(&public);
        let script: zcash_transparent::address::Script = address.script().into();
        if utxo.script != script.0.0 {
            return Err(MiningKeyError::Utxo("script does not match miner key"));
        }
        let spend_height = utxo
            .height
            .checked_add(COINBASE_MATURITY_BLOCKS)
            .ok_or(MiningKeyError::Utxo("maturity height overflow"))?;
        if u64::from(target_height) < spend_height {
            return Err(MiningKeyError::Immature(spend_height));
        }
        let txid: [u8; 32] = utxo
            .txid
            .as_slice()
            .try_into()
            .map_err(|_| MiningKeyError::Utxo("transaction ID has wrong length"))?;
        let index =
            u32::try_from(utxo.index).map_err(|_| MiningKeyError::Utxo("negative output index"))?;
        let value = u64::try_from(utxo.value_zat)
            .map_err(|_| MiningKeyError::Utxo("negative output value"))?;
        let value =
            Zatoshis::from_u64(value).map_err(|error| MiningKeyError::Sweep(error.to_string()))?;
        let target = BlockHeight::from_u32(target_height);
        let fee_rule = zip317::FeeRule::standard();
        let fee = Zatoshis::from_u64(ironwood_sweep_fee_for_chain(self.chain, target_height)?)
            .map_err(|error| MiningKeyError::Sweep(error.to_string()))?;
        let amount = (value - fee)
            .ok_or_else(|| MiningKeyError::Sweep("output does not cover fee".to_owned()))?;

        let mut signing_set = TransparentSigningSet::new();
        let pubkey = signing_set.add_key(self.secret);
        let mut builder = Builder::new(
            self.chain,
            target,
            BuildConfig::Standard {
                sapling_anchor: None,
                orchard_anchor: None,
                ironwood_anchor: Some(anchor),
                orchard_padding: BundlePadding::DEFAULT,
                ironwood_padding: BundlePadding::DEFAULT,
            },
        )
        .with_expiry_height(crate::wallet::expiry::tx_expiry_height(&self.chain, target));
        builder
            .add_transparent_p2pkh_input(
                pubkey,
                OutPoint::new(txid, index),
                TxOut::new(value, script),
            )
            .map_err(|error| MiningKeyError::Sweep(error.to_string()))?;
        builder
            .add_ironwood_output::<std::convert::Infallible>(
                None,
                recipient,
                amount,
                MemoBytes::empty(),
            )
            .map_err(|error| MiningKeyError::Sweep(error.to_string()))?;
        let prover = crate::wallet::utils::LazySaplingProver::default();
        let result = builder
            .build(
                &signing_set,
                &[],
                &[],
                crate::utils::system_rng(),
                &prover,
                &prover,
                &fee_rule,
            )
            .map_err(|error| MiningKeyError::Sweep(error.to_string()))?;
        let mut raw = Vec::new();
        result
            .transaction()
            .write(&mut raw)
            .map_err(|error| MiningKeyError::Sweep(error.to_string()))?;
        Ok(raw)
    }

    pub async fn balance(&self, uri: Uri) -> Result<MiningBalance, MiningKeyError> {
        let network = zl_netutils::NetworkAccess::online(uri).await?;
        verify_catecash_indexer(&network, self.chain).await?;
        let mut indexer = network
            .indexer()
            .ok_or(zl_netutils::GetClientError::NoIndexerUri)?
            .clone();
        let tip = indexer
            .get_lightd_info(DEFAULT_REQUEST_TIMEOUT)
            .await?
            .block_height;
        let confirmed_zat = indexer
            .get_catecash_taddress_balance(
                AddressList {
                    addresses: vec![self.address.clone()],
                },
                DEFAULT_REQUEST_TIMEOUT,
            )
            .await?;
        let reply = indexer
            .get_address_utxos(
                GetAddressUtxosArg {
                    addresses: vec![self.address.clone()],
                    start_height: 0,
                    max_entries: 0,
                },
                DEFAULT_REQUEST_TIMEOUT,
            )
            .await?;
        let mut summed_utxos = 0u128;
        let mut spendable_next_block_zat = 0u128;
        let mut next_spend_height = None;
        for utxo in reply.address_utxos {
            if utxo.address != self.address || utxo.height > tip {
                return Err(MiningKeyError::Utxo("address or height differs"));
            }
            let amount = u64::try_from(utxo.value_zat)
                .map_err(|_| MiningKeyError::Utxo("negative amount"))?;
            summed_utxos = summed_utxos
                .checked_add(u128::from(amount))
                .ok_or(MiningKeyError::Utxo("balance overflow"))?;
            let spend_height = utxo
                .height
                .checked_add(COINBASE_MATURITY_BLOCKS)
                .ok_or(MiningKeyError::Utxo("maturity height overflow"))?;
            if tip.saturating_add(1) >= spend_height {
                spendable_next_block_zat = spendable_next_block_zat
                    .checked_add(u128::from(amount))
                    .ok_or(MiningKeyError::Utxo("balance overflow"))?;
            } else {
                next_spend_height = Some(
                    next_spend_height
                        .map_or(spend_height, |earliest: u64| earliest.min(spend_height)),
                );
            }
        }
        if summed_utxos != confirmed_zat {
            return Err(MiningKeyError::Utxo("balance differs from UTXO sum"));
        }
        Ok(MiningBalance {
            confirmed_zat,
            spendable_next_block_zat,
            next_spend_height,
            tip_height: tip,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zcash_protocol::consensus::NetworkType;
    use zl_netutils::lightwallet_protocol::BlockId;

    #[test]
    fn imports_miner_wif_and_rejects_other_networks() {
        let mut bytes = vec![zcash_protocol::constants::catecash_devnet::B58_SECRET_KEY_PREFIX[0]];
        bytes.extend([0x42; 32]);
        bytes.push(COMPRESSED_PUBLIC_KEY_MARKER);
        let wif = bs58::encode(&bytes).with_check().into_string();
        let key = MiningKey::import_wif(&wif).expect("fixed miner key imports");
        assert!(key.address().starts_with("yP"));

        bytes[0] = 0x80;
        let zcash_wif = bs58::encode(bytes).with_check().into_string();
        assert!(matches!(
            MiningKey::import_wif(&zcash_wif),
            Err(MiningKeyError::Format)
        ));
    }

    #[test]
    fn mining_keys_require_the_selected_catecash_network() {
        for (chain, prefix, address_prefix) in [
            (ChainType::CatecashDevnet, 0xf0, "yP"),
            (ChainType::CatecashTestnet, 0xd3, "EM"),
            (ChainType::CatecashMainnet, 0xd2, "Ca"),
        ] {
            let mut bytes = vec![prefix];
            bytes.extend([0x42; 32]);
            bytes.push(COMPRESSED_PUBLIC_KEY_MARKER);
            let wif = bs58::encode(bytes).with_check().into_string();
            let key = MiningKey::import_wif_for_chain(&wif, chain)
                .expect("fixed key matches the selected Catecash network");
            assert!(key.address().starts_with(address_prefix));
            for other in [
                ChainType::CatecashDevnet,
                ChainType::CatecashTestnet,
                ChainType::CatecashMainnet,
            ] {
                if other != chain {
                    assert!(matches!(
                        MiningKey::import_wif_for_chain(&wif, other),
                        Err(MiningKeyError::Format)
                    ));
                }
            }
        }
    }

    #[test]
    fn sweep_rejects_an_immature_or_wrong_output() {
        let mut bytes = vec![zcash_protocol::constants::catecash_devnet::B58_SECRET_KEY_PREFIX[0]];
        bytes.extend([0x42; 32]);
        bytes.push(COMPRESSED_PUBLIC_KEY_MARKER);
        let wif = bs58::encode(bytes).with_check().into_string();
        let key = MiningKey::import_wif(&wif).expect("fixed miner WIF");
        let public = PublicKey::from_secret_key(&key.secret);
        let script: zcash_transparent::address::Script =
            TransparentAddress::from_pubkey(&public).script().into();
        let utxo = GetAddressUtxosReply {
            address: key.address().to_owned(),
            txid: vec![0x11; 32],
            index: 0,
            script: script.0.0,
            value_zat: 500_000_000_000,
            height: 1,
        };
        let spending_key =
            orchard::keys::SpendingKey::from_bytes([0x42; 32]).expect("fixed Orchard secret");
        let recipient = orchard::keys::FullViewingKey::from(&spending_key)
            .address_at(0u32, orchard::keys::Scope::External);
        let anchor = orchard::Anchor::empty_tree();
        assert!(matches!(
            key.build_ironwood_sweep(&utxo, 240, recipient, anchor),
            Err(MiningKeyError::Immature(241))
        ));
        let mut wrong = utxo.clone();
        wrong.script.clear();
        assert!(matches!(
            key.build_ironwood_sweep(&wrong, 241, recipient, anchor),
            Err(MiningKeyError::Utxo("script does not match miner key"))
        ));
        let raw = key
            .build_ironwood_sweep(&utxo, 241, recipient, anchor)
            .expect("mature reward signs a Catecash Ironwood sweep");
        let tx = zcash_primitives::transaction::Transaction::read(
            raw.as_slice(),
            zcash_protocol::consensus::BranchId::Catecash,
        )
        .expect("Catecash transaction parses");
        assert!(tx.ironwood_bundle().is_some());
        assert_eq!(
            tx.transparent_bundle()
                .expect("transparent input")
                .vin
                .len(),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires a local Catecash Zaino connection and the miner key file"]
    async fn hosted_miner_key_matches_reward_and_maturity() {
        let path = std::env::var("CATECASH_MINER_KEY_FILE").expect("miner key file path");
        let contents = std::fs::read_to_string(path).expect("read miner key file");
        let document = json::parse(&contents).expect("parse miner key file");
        let wif = document["private_key_wif"].as_str().expect("WIF field");
        let key = MiningKey::import_wif(wif).expect("import Catecash miner WIF");
        assert_eq!(
            key.address(),
            document["address"].as_str().expect("address field")
        );
        let uri: http::Uri = std::env::var("CATECASH_INDEXER_URI")
            .unwrap_or_else(|_| "http://127.0.0.1:34347".to_owned())
            .parse()
            .expect("indexer URI");
        let balance = key.balance(uri.clone()).await.expect("query miner balance");
        assert!(balance.confirmed_zat > 0);
        assert!(balance.tip_height < balance.next_spend_height.unwrap_or(u64::MAX));
        let network = zl_netutils::NetworkAccess::online(uri)
            .await
            .expect("connect to Catecash indexer");
        let mut indexer = network.indexer().expect("online indexer").clone();
        let outputs = indexer
            .get_address_utxos(
                GetAddressUtxosArg {
                    addresses: vec![key.address().to_owned()],
                    start_height: 0,
                    max_entries: 0,
                },
                DEFAULT_REQUEST_TIMEOUT,
            )
            .await
            .expect("fetch live mining output");
        let first = outputs.address_utxos.first().expect("mining reward output");
        let tree = indexer
            .get_tree_state(
                BlockId {
                    height: balance.tip_height,
                    hash: Vec::new(),
                },
                DEFAULT_REQUEST_TIMEOUT,
            )
            .await
            .expect("fetch Ironwood tree state");
        assert_eq!(
            tree.ironwood_tree, "000000",
            "Ironwood anchor must be empty"
        );
        let recipient = match std::env::var("CATECASH_SWEEP_ADDRESS") {
            Ok(encoded) => {
                let address = ZcashAddress::try_from_encoded(&encoded)
                    .expect("Catecash destination parses")
                    .convert_if_network::<zcash_keys::address::Address>(NetworkType::CatecashDevnet)
                    .expect("destination belongs to Catecash devnet");
                match address {
                    zcash_keys::address::Address::Unified(unified) => unified
                        .orchard()
                        .copied()
                        .expect("Ironwood uses the unified Orchard receiver"),
                    _ => panic!("Catecash sweep destination must be unified"),
                }
            }
            Err(_) => {
                let recipient_key = orchard::keys::SpendingKey::from_bytes([0x42; 32])
                    .expect("fixed test recipient key");
                orchard::keys::FullViewingKey::from(&recipient_key)
                    .address_at(0u32, orchard::keys::Scope::External)
            }
        };
        let target_height =
            (first.height + COINBASE_MATURITY_BLOCKS).max(balance.tip_height.saturating_add(1));
        let raw = key
            .build_ironwood_sweep(
                first,
                u32::try_from(target_height).expect("devnet maturity fits block height"),
                recipient,
                orchard::Anchor::empty_tree(),
            )
            .expect("real coinbase output signs without broadcasting");
        let tx = zcash_primitives::transaction::Transaction::read(
            raw.as_slice(),
            zcash_protocol::consensus::BranchId::Catecash,
        )
        .expect("signed Catecash transaction parses");
        assert!(tx.ironwood_bundle().is_some());
        if let Ok(path) = std::env::var("CATECASH_SWEEP_RAW_FILE") {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt as _;

            assert!(
                std::env::var("CATECASH_SWEEP_ADDRESS").is_ok(),
                "an explicit backed-up destination is required for export"
            );
            assert!(
                balance.tip_height.saturating_add(1) >= first.height + COINBASE_MATURITY_BLOCKS,
                "do not export an immature spend"
            );
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .expect("create owner-only signed transaction file");
            file.write_all(hex::encode(raw).as_bytes())
                .expect("write signed transaction hex");
        }
        println!("miner address verified; balance: {balance:?}");
    }

    #[tokio::test]
    #[ignore = "requires a funded Catecash devnet wallet and local Zaino connection"]
    async fn hosted_ironwood_wallet_receives_mining_sweep() {
        use crate::config::{ClientConfig, WalletConfig};
        use crate::lightclient::LightClient;

        let wallet_dir =
            std::env::var("CATECASH_WALLET_DIR").expect("backed-up Catecash wallet directory");
        let uri: http::Uri = std::env::var("CATECASH_INDEXER_URI")
            .unwrap_or_else(|_| "http://127.0.0.1:34347".to_owned())
            .parse()
            .expect("indexer URI");
        let config = ClientConfig::builder()
            .set_chain_type(ChainType::CatecashDevnet)
            .set_wallet_dir(wallet_dir.into())
            .set_wallet_config(WalletConfig::Read)
            .set_indexer_uri(uri)
            .build()
            .expect("Catecash wallet configuration");
        let mut client = LightClient::new(config, false)
            .await
            .expect("read existing Catecash wallet");
        client
            .sync_to_tip_and_await()
            .await
            .expect("sync Catecash wallet through the mining sweep");
        let balance = client
            .account_balance(zip32::AccountId::ZERO)
            .await
            .expect("wallet account balance");
        let received = balance
            .total_ironwood_balance
            .expect("Ironwood balance available")
            .into_u64();
        if std::env::var_os("CATECASH_EXPECT_EMPTY_WALLET").is_some() {
            assert_eq!(received, 0, "wallet received an unexpected Ironwood note");
        } else {
            assert!(received > 0, "the mining sweep must appear in Ironwood");
        }
        client.flush().await.expect("persist synced wallet");
        println!("Catecash wallet received {received} zats in Ironwood");
    }
}
