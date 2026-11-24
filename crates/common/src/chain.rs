use zingo_common_components::protocol::ActivationHeights;

/// The network types a lightclient can connect to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainType {
    /// Mainnet
    Mainnet,
    /// Testnet
    Testnet,
    /// Catecash development network
    CatecashDevnet,
    /// Catecash testnet version 1.
    CatecashTestnet,
    /// Catecash mainnet candidate.
    CatecashMainnet,
    /// Regtest
    Regtest(ActivationHeights),
}

impl std::fmt::Display for ChainType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let chain = match self {
            ChainType::Mainnet => "mainnet",
            ChainType::Testnet => "testnet",
            ChainType::CatecashDevnet => "catecash-devnet",
            ChainType::CatecashTestnet => "catecash-testnet",
            ChainType::CatecashMainnet => "catecash-mainnet",
            ChainType::Regtest(_) => "regtest",
        };
        write!(f, "{chain}")
    }
}

impl TryFrom<&str> for ChainType {
    type Error = InvalidChainType;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "mainnet" | "main" => Ok(ChainType::Mainnet),
            "testnet" | "test" => Ok(ChainType::Testnet),
            "catecash-devnet" | "CatecashDevnet" => Ok(ChainType::CatecashDevnet),
            "catecash-testnet" | "CatecashTestnet" => Ok(ChainType::CatecashTestnet),
            "catecash-mainnet" | "CatecashMainnet" => Ok(ChainType::CatecashMainnet),
            "regtest" => Ok(ChainType::Regtest(ActivationHeights::default())),
            _ => Err(InvalidChainType(value.to_string())),
        }
    }
}

pub(crate) mod consealed {
    use zcash_protocol::consensus::{
        BlockHeight, BranchId, MAIN_NETWORK, NetworkType, NetworkUpgrade, Parameters, TEST_NETWORK,
    };

    use super::ChainType;

    impl Parameters for ChainType {
        fn network_type(&self) -> NetworkType {
            match self {
                ChainType::Mainnet => NetworkType::Main,
                ChainType::Testnet => NetworkType::Test,
                ChainType::CatecashDevnet => NetworkType::CatecashDevnet,
                ChainType::CatecashTestnet => NetworkType::CatecashTestnet,
                ChainType::CatecashMainnet => NetworkType::CatecashMainnet,
                ChainType::Regtest(_) => NetworkType::Regtest,
            }
        }

        fn activation_height(&self, nu: NetworkUpgrade) -> Option<BlockHeight> {
            match self {
                ChainType::Mainnet => MAIN_NETWORK.activation_height(nu),
                ChainType::Testnet => TEST_NETWORK.activation_height(nu),
                ChainType::CatecashDevnet => Some(BlockHeight::from_u32(1)),
                ChainType::CatecashTestnet => Some(BlockHeight::from_u32(1)),
                ChainType::CatecashMainnet => Some(BlockHeight::from_u32(1)),
                ChainType::Regtest(activation_heights) => match nu {
                    NetworkUpgrade::Overwinter => {
                        activation_heights.overwinter().map(BlockHeight::from_u32)
                    }
                    NetworkUpgrade::Sapling => {
                        activation_heights.sapling().map(BlockHeight::from_u32)
                    }
                    NetworkUpgrade::Blossom => {
                        activation_heights.blossom().map(BlockHeight::from_u32)
                    }
                    NetworkUpgrade::Heartwood => {
                        activation_heights.heartwood().map(BlockHeight::from_u32)
                    }
                    NetworkUpgrade::Canopy => {
                        activation_heights.canopy().map(BlockHeight::from_u32)
                    }
                    NetworkUpgrade::Nu5 => activation_heights.nu5().map(BlockHeight::from_u32),
                    NetworkUpgrade::Nu6 => activation_heights.nu6().map(BlockHeight::from_u32),
                    NetworkUpgrade::Nu6_1 => activation_heights.nu6_1().map(BlockHeight::from_u32),
                    NetworkUpgrade::Nu6_2 => activation_heights.nu6_2().map(BlockHeight::from_u32),
                    NetworkUpgrade::Nu6_3 => activation_heights.nu6_3().map(BlockHeight::from_u32),
                    NetworkUpgrade::Nu7 => activation_heights.nu7().map(BlockHeight::from_u32),
                },
            }
        }

        fn branch_id_override(&self, height: BlockHeight) -> Option<BranchId> {
            if matches!(self, ChainType::CatecashDevnet) && height >= BlockHeight::from_u32(1) {
                Some(BranchId::Catecash)
            } else if matches!(self, ChainType::CatecashTestnet)
                && height >= BlockHeight::from_u32(1)
            {
                Some(BranchId::CatecashTestnet)
            } else if matches!(self, ChainType::CatecashMainnet)
                && height >= BlockHeight::from_u32(1)
            {
                Some(BranchId::CatecashMainnet)
            } else {
                None
            }
        }
    }
}

/// Invalid chain type.
#[derive(thiserror::Error, Debug)]
#[error(
    "Invalid chain type '{0}'. Expected one of: 'mainnet', 'testnet', 'regtest', 'catecash-devnet', 'catecash-testnet' or 'catecash-mainnet'."
)]
pub struct InvalidChainType(String);

#[cfg(test)]
mod tests {
    use super::ChainType;
    use zcash_protocol::consensus::{BlockHeight, BranchId, NetworkType, Parameters};
    use zingo_common_components::protocol::ActivationHeights;

    #[test]
    fn test_chaintypes_from_strings() {
        assert_eq!(
            ChainType::try_from("mainnet").expect("'mainnet' names the main network"),
            ChainType::Mainnet
        );
        assert_eq!(
            ChainType::try_from("testnet").expect("'testnet' names the test network"),
            ChainType::Testnet
        );
        // the following 2 variants match the string from get_lightd_info, from the LightdInfo struct
        // as referenced in librustzcash/zcash_client_backend/src/proto/service.rs 17a4830c70
        // and provided by an indexer (previously called lightwallet)
        assert_eq!(
            ChainType::try_from("main").expect("'main' names the main network"),
            ChainType::Mainnet
        );
        assert_eq!(
            ChainType::try_from("test").expect("'test' names the test network"),
            ChainType::Testnet
        );
        assert_eq!(
            ChainType::try_from("regtest").expect("'regtest' names the regtest network"),
            ChainType::Regtest(ActivationHeights::default())
        );
        assert!(ChainType::try_from("invalid").is_err());
        let err = ChainType::try_from("badvalue").expect_err("'badvalue' names no network");
        assert_eq!(err.0, "badvalue");
        assert!(ChainType::try_from("").is_err());
        assert!(ChainType::try_from("Mainnet").is_err());
        assert!(ChainType::try_from("MAINNET").is_err());
        assert!(ChainType::try_from("Testnet").is_err());
        assert!(ChainType::try_from("Regtest").is_err());
        assert!(ChainType::try_from("mainnet ").is_err()); // trailing space
        assert!(ChainType::try_from(" mainnet").is_err()); // leading space
        assert!(ChainType::try_from("regtест").is_err()); // lookalike chars
    }

    #[test]
    fn catecash_devnet_uses_distinct_keys_addresses_and_branch() {
        use crate::keys::{
            UnifiedKeyStore, decode_address,
            transparent::{TransparentScope, encode_address},
        };
        use zcash_address::ToAddress as _;
        use zcash_transparent::keys::NonHardenedChildIndex;

        let chain = ChainType::CatecashDevnet;
        assert_eq!(
            ChainType::try_from("catecash-devnet").expect("valid Catecash name"),
            chain
        );
        assert_eq!(chain.to_string(), "catecash-devnet");
        assert_eq!(chain.network_type(), NetworkType::CatecashDevnet);
        assert_eq!(
            BranchId::for_height(&chain, BlockHeight::from_u32(1)),
            BranchId::Catecash
        );
        assert_eq!(u32::from(BranchId::Catecash), 0xea3b_eeb4);

        let key = UnifiedKeyStore::new_from_seed(chain, &[7u8; 64], zip32::AccountId::ZERO)
            .expect("fixed seed derives a Catecash account");
        let child = NonHardenedChildIndex::from_index(0).expect("zero is nonhardened");
        let address = key
            .generate_transparent_address(child, TransparentScope::External)
            .expect("account has a transparent receiver");
        let encoded = encode_address(&chain, address);
        assert!(encoded.starts_with("yP"));
        assert!(decode_address(&chain, &encoded).is_ok());
        assert!(decode_address(&ChainType::Mainnet, &encoded).is_err());
        assert!(decode_address(&chain, "t1Smwq8rE6dLRt4J9fDpi5X5K7Q6VL6BnGD").is_err());

        let node_vector =
            zcash_address::ZcashAddress::from_transparent_p2pkh(chain.network_type(), [0x42; 20]);
        assert_eq!(
            node_vector.to_string(),
            "yPeC241BhV8S3oRB58xPobSFzJXkhYxwhKK"
        );
    }

    #[test]
    fn catecash_testnet_rejects_devnet_and_zcash_addresses() {
        use crate::keys::decode_address;
        use zcash_address::ToAddress as _;

        let chain = ChainType::CatecashTestnet;
        assert_eq!(ChainType::try_from("catecash-testnet").unwrap(), chain);
        assert_eq!(chain.to_string(), "catecash-testnet");
        assert_eq!(chain.network_type(), NetworkType::CatecashTestnet);
        assert_eq!(
            BranchId::for_height(&chain, BlockHeight::from_u32(1)),
            BranchId::CatecashTestnet
        );
        assert_eq!(u32::from(BranchId::CatecashTestnet), 0xb80d_ae22);

        let address =
            zcash_address::ZcashAddress::from_transparent_p2pkh(chain.network_type(), [0x42; 20])
                .to_string();
        assert_eq!(address, "EMHKsQKxJbucJ4NBdJ9xZ9ZdDayLWutX19e");
        assert!(decode_address(&chain, &address).is_ok());
        assert!(decode_address(&ChainType::CatecashDevnet, &address).is_err());
        assert!(decode_address(&ChainType::Testnet, &address).is_err());
    }

    #[test]
    fn catecash_mainnet_candidate_has_its_own_keys_and_addresses() {
        use crate::keys::decode_address;
        use zcash_address::ToAddress as _;

        let chain = ChainType::CatecashMainnet;
        assert_eq!(ChainType::try_from("catecash-mainnet").ok(), Some(chain));
        assert_eq!(chain.network_type(), NetworkType::CatecashMainnet);
        assert_eq!(
            BranchId::for_height(&chain, BlockHeight::from_u32(1)),
            BranchId::CatecashMainnet
        );
        assert_eq!(u32::from(BranchId::CatecashMainnet), 0x5bc6_8392);

        let address =
            zcash_address::ZcashAddress::from_transparent_p2pkh(chain.network_type(), [0x42; 20])
                .to_string();
        assert_eq!(address, "CaGoTLXeYqiyzhQVT2kgy1TwpTqP2okCHZX");
        assert!(decode_address(&chain, &address).is_ok());
        assert!(decode_address(&ChainType::CatecashDevnet, &address).is_err());
        assert!(decode_address(&ChainType::CatecashTestnet, &address).is_err());
        assert!(decode_address(&ChainType::Mainnet, &address).is_err());
    }
}
