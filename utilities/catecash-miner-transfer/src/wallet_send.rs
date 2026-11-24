use std::path::PathBuf;

mod network;

use zcash_address::ZcashAddress;
use zcash_client_backend::zip321::{Payment, TransactionRequest};
use zcash_protocol::value::Zatoshis;
use zingolib::config::{ChainType, ClientConfig, WalletConfig};
use zingolib::lightclient::LightClient;

const ZAT_PER_CATE: u64 = 100_000_000;

struct Args {
    chain: ChainType,
    key_file: PathBuf,
    to: String,
    amount_zat: u64,
    indexer: http::Uri,
    send: bool,
}

fn parse_amount(value: &str) -> Result<u64, String> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || fraction.len() > 8
        || !fraction.bytes().all(|c| c.is_ascii_digit())
    {
        return Err("amount must be a positive decimal with at most eight places".to_owned());
    }
    let whole = whole.parse::<u64>().map_err(|e| e.to_string())?;
    let fraction = format!("{fraction:0<8}")
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    let amount = whole
        .checked_mul(ZAT_PER_CATE)
        .and_then(|n| n.checked_add(fraction))
        .ok_or("amount exceeds the CATE range")?;
    if amount == 0 {
        return Err("amount must be positive".to_owned());
    }
    Ok(amount)
}

fn parse_args() -> Result<Args, String> {
    let mut key_file = None;
    let mut to = None;
    let mut amount_zat = None;
    let mut indexer = None;
    let mut send = false;
    let mut chain = None;
    let mut input = std::env::args().skip(1);
    while let Some(flag) = input.next() {
        if flag == "--send" {
            send = true;
            continue;
        }
        let value = input
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--network" => chain = Some(network::parse_chain(&value)?),
            "--key-file" => key_file = Some(PathBuf::from(value)),
            "--to" => to = Some(value),
            "--amount" => amount_zat = Some(parse_amount(&value)?),
            "--indexer" => indexer = Some(value.parse::<http::Uri>().map_err(|e| e.to_string())?),
            _ => return Err(format!("unknown option: {flag}")),
        }
    }
    Ok(Args {
        chain: chain.ok_or("--network is required")?,
        key_file: key_file.ok_or("--key-file is required")?,
        to: to.ok_or("--to is required")?,
        amount_zat: amount_zat.ok_or("--amount is required")?,
        indexer: indexer.ok_or("--indexer is required")?,
        send,
    })
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    let recipient = ZcashAddress::try_from_encoded(&args.to).map_err(|e| e.to_string())?;
    recipient
        .clone()
        .convert_if_network::<zcash_keys::address::Address>(network::network_type(args.chain))
        .map_err(|e| e.to_string())?;
    let wallet_dir = args.key_file.parent().ok_or("key file has no directory")?;
    let wallet_name = args
        .key_file
        .file_name()
        .ok_or("key file has no name")?
        .to_str()
        .ok_or("key file name is not UTF-8")?;
    if !args.key_file.is_file() {
        return Err("wallet key file does not exist".to_owned());
    }
    let config = ClientConfig::builder()
        .set_chain_type(args.chain)
        .set_wallet_dir(wallet_dir.to_path_buf())
        .set_wallet_name(wallet_name.to_owned())
        .set_wallet_config(WalletConfig::Read)
        .set_indexer_uri(args.indexer)
        .build()
        .map_err(|e| e.to_string())?;
    let mut client = LightClient::new(config, false)
        .await
        .map_err(|e| e.to_string())?;
    client
        .sync_to_tip_and_await()
        .await
        .map_err(|e| e.to_string())?;
    let balance = client
        .account_balance(zip32::AccountId::ZERO)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "network: {}\nrecipient: {}\namount: {}.{:08} CATE\nconfirmed Ironwood balance: {:?}\nmode: {}",
        args.chain,
        args.to,
        args.amount_zat / ZAT_PER_CATE,
        args.amount_zat % ZAT_PER_CATE,
        balance.confirmed_ironwood_balance,
        if args.send { "broadcast" } else { "preview" }
    );
    let result = if args.send {
        let amount = Zatoshis::from_u64(args.amount_zat).map_err(|e| e.to_string())?;
        let payment = Payment::new(recipient, Some(amount), None, None, None, vec![])
            .map_err(|e| e.to_string())?;
        let request = TransactionRequest::new(vec![payment]).map_err(|e| e.to_string())?;
        Some(
            client
                .quick_send(request, zip32::AccountId::ZERO, false)
                .await,
        )
    } else {
        None
    };
    client.flush().await.map_err(|e| e.to_string())?;
    if let Some(result) = result {
        let txids = result.map_err(|e| format!("{e:?}; check wallet history before retrying"))?;
        for txid in txids.iter() {
            println!("txid: {txid}");
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    run().await
}

#[cfg(test)]
mod tests {
    use super::parse_amount;

    #[test]
    fn decimal_amounts_use_exact_catemoto() {
        assert_eq!(parse_amount("100"), Ok(10_000_000_000));
        assert_eq!(parse_amount("0.00000001"), Ok(1));
        assert!(parse_amount("0").is_err());
        assert!(parse_amount("1.000000001").is_err());
    }
}
