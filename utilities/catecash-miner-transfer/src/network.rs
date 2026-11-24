use zcash_protocol::consensus::{NetworkType, Parameters};
use zingolib::config::ChainType;

pub fn parse_chain(value: &str) -> Result<ChainType, String> {
    match value {
        "devnet" | "catecash-devnet" => Ok(ChainType::CatecashDevnet),
        "testnet" | "catecash-testnet" => Ok(ChainType::CatecashTestnet),
        "mainnet" | "catecash-mainnet" => Ok(ChainType::CatecashMainnet),
        _ => Err("--network must be devnet, testnet, or mainnet".to_owned()),
    }
}

pub fn network_type(chain: ChainType) -> NetworkType {
    chain.network_type()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_names_select_catecash_chains() {
        assert_eq!(parse_chain("devnet"), Ok(ChainType::CatecashDevnet));
        assert_eq!(parse_chain("testnet"), Ok(ChainType::CatecashTestnet));
        assert_eq!(parse_chain("mainnet"), Ok(ChainType::CatecashMainnet));
        assert!(parse_chain("zcash-testnet").is_err());
    }
}
