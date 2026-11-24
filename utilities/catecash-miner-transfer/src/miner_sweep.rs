use std::path::PathBuf;

mod network;

use incrementalmerkletree::frontier::CommitmentTree;
use orchard::tree::MerkleHashOrchard;
use zcash_address::ZcashAddress;
use zcash_primitives::merkle_tree::read_commitment_tree;
use zingolib::catecash_mining::{
    COINBASE_MATURITY_BLOCKS, MiningKey, ironwood_sweep_fee_for_chain,
};
use zingolib::config::ChainType;
use zl_netutils::lightwallet_protocol::{BlockId, GetAddressUtxosArg, RawTransaction};
use zl_netutils::{Indexer as _, TransparentIndexer as _};

const ZAT_PER_CATE: u64 = 100_000_000;

struct Args {
    chain: ChainType,
    key_file: PathBuf,
    to: String,
    indexer: http::Uri,
    send: bool,
}

fn format_amount(zat: u64) -> String {
    format!("{}.{:08}", zat / ZAT_PER_CATE, zat % ZAT_PER_CATE)
}

fn parse_args() -> Result<Args, String> {
    let mut key_file = None;
    let mut to = None;
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
            "--indexer" => indexer = Some(value.parse::<http::Uri>().map_err(|e| e.to_string())?),
            _ => return Err(format!("unknown option: {flag}")),
        }
    }
    Ok(Args {
        chain: chain.ok_or("--network is required")?,
        key_file: key_file.ok_or("--key-file is required")?,
        to: to.ok_or("--to is required")?,
        indexer: indexer.ok_or("--indexer is required")?,
        send,
    })
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&args.key_file).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let wif = document["private_key_wif"]
        .as_str()
        .ok_or("key file lacks private_key_wif")?;
    let key = MiningKey::import_wif_for_chain(wif, args.chain).map_err(|e| e.to_string())?;
    let recipient = ZcashAddress::try_from_encoded(&args.to)
        .map_err(|e| e.to_string())?
        .convert_if_network::<zcash_keys::address::Address>(network::network_type(args.chain))
        .map_err(|e| e.to_string())?;
    let zcash_keys::address::Address::Unified(unified) = recipient else {
        return Err(format!(
            "destination must be a {} unified address",
            args.chain
        ));
    };
    let receiver = unified
        .orchard()
        .copied()
        .ok_or("destination lacks an Ironwood receiver")?;
    let balance = key
        .balance(args.indexer.clone())
        .await
        .map_err(|e| e.to_string())?;
    let target_height = u32::try_from(balance.tip_height + 1).map_err(|e| e.to_string())?;
    let network = zl_netutils::NetworkAccess::online(args.indexer)
        .await
        .map_err(|e| e.to_string())?;
    let mut indexer = network.indexer().ok_or("indexer unavailable")?.clone();
    let outputs = indexer
        .get_address_utxos(
            GetAddressUtxosArg {
                addresses: vec![key.address().to_owned()],
                start_height: 0,
                max_entries: 0,
            },
            zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT,
        )
        .await
        .map_err(|e| e.to_string())?;
    let utxo = outputs
        .address_utxos
        .iter()
        .find(|utxo| {
            utxo.height.saturating_add(COINBASE_MATURITY_BLOCKS) <= u64::from(target_height)
        })
        .ok_or("no mature mining reward is available")?;
    let tree = indexer
        .get_tree_state(
            BlockId {
                height: balance.tip_height,
                hash: Vec::new(),
            },
            zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT,
        )
        .await
        .map_err(|e| e.to_string())?;
    let tree_bytes = hex::decode(&tree.ironwood_tree).map_err(|e| e.to_string())?;
    let frontier: CommitmentTree<MerkleHashOrchard, 32> =
        read_commitment_tree(tree_bytes.as_slice()).map_err(|e| e.to_string())?;
    let anchor = orchard::Anchor::from(frontier.root());
    let raw = key
        .build_ironwood_sweep(utxo, target_height, receiver, anchor)
        .map_err(|e| e.to_string())?;
    let tx = zcash_primitives::transaction::Transaction::read(
        raw.as_slice(),
        zcash_protocol::consensus::BranchId::Catecash,
    )
    .map_err(|e| e.to_string())?;
    let transparent = tx
        .transparent_bundle()
        .ok_or("signed transaction lacks transparent input")?;
    if !transparent.vout.is_empty() {
        return Err("coinbase sweep unexpectedly contains a transparent output".to_owned());
    }
    let input = u64::try_from(utxo.value_zat).map_err(|e| e.to_string())?;
    let fee = ironwood_sweep_fee_for_chain(args.chain, target_height).map_err(|e| e.to_string())?;
    let amount = input.checked_sub(fee).ok_or("input does not cover fee")?;
    println!(
        "network: {}\nminer: {}\nrecipient: {}\namount: {} CATE\nfee: {} CATE\nreward height: {}\ntarget height: {}\ninput: {}:{}\nmode: {}",
        args.chain,
        key.address(),
        args.to,
        format_amount(amount),
        format_amount(fee),
        utxo.height,
        target_height,
        hex::encode(&utxo.txid),
        utxo.index,
        if args.send { "broadcast" } else { "preview" }
    );
    if args.send {
        let txid = indexer
            .send_transaction(
                RawTransaction {
                    data: raw,
                    height: u64::from(u32::from(tx.into_data().expiry_height())),
                },
                zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT,
            )
            .await
            .map_err(|e| e.to_string())?;
        println!("txid: {txid}");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    run().await
}
