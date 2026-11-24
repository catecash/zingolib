# Catecash transfer utilities

`catecash-miner-sweep` spends one mature coinbase reward into a shielded address on the selected Catecash network. Coinbase spends require shielded-only outputs, so the entire reward minus its fee moves to the recipient. `catecash-wallet-send` sends an exact amount from a funded shielded wallet key file. Both commands require `--network devnet`, `--network testnet`, or `--network mainnet` and an indexer for that chain. Mainnet remains a candidate network.

Both commands preview by default. Add `--send` to broadcast. The key file and wallet directory stay on the local machine; neither command prints private keys.

```sh
cargo run -p catecash-miner-transfer --bin catecash-miner-sweep -- \
  --network devnet \
  --key-file "$HOME/.catecash/devnet/miner-key.json" \
  --to <staging-shielded-address> \
  --indexer https://wallet-devnet.catecash.com:443

cargo run -p catecash-miner-transfer --bin catecash-wallet-send -- \
  --network devnet \
  --key-file "$HOME/.catecash/devnet/transfer-staging/catecash-wallet.dat" \
  --to <destination-address> \
  --amount 100 \
  --indexer https://wallet-devnet.catecash.com:443
```

The miner key file is JSON with `private_key_wif` encoded for the selected network. The wallet file contains private spending keys for that network. Back up the staging wallet recovery phrase before sweeping funds to it. Wait for the sweep to confirm and become spendable before running the exact-amount sender. `--amount` accepts CATE with up to eight decimal places. After a send error, inspect wallet history before retrying.

The testnet gateway is `https://wallet-testnet.catecash.com:443`. The hosted testnet miner pays the controlled address `EMDhJhRcLFL7ccnj53pbSnyB3eQRyW9dcH4` from block 351 onward. Its local key is `~/.catecash/testnet/miner-key.json`, and the staging wallet is `~/.catecash/testnet/transfer-staging/catecash-testnet-wallet.dat`. Coinbase rewards require 240-block maturity, so the first reward can be swept in block 591 or later. Candidate mainnet mining wallets use mainnet address and WIF prefixes, so their balances cannot fund a testnet transfer.

For the hosted testnet, preview the sweep after maturity:

```sh
cargo run -p catecash-miner-transfer --bin catecash-miner-sweep -- \
  --network testnet \
  --key-file "$HOME/.catecash/testnet/miner-key.json" \
  --to catet1xh2lc70dr5dumddc40rpdlfz2p5s5raw874ptlkak8xcjd9shmzs6hdmz4u0e7yjq66dgcfu8ul4095jwg8ayyw3sxv98cefscpdvzl9 \
  --indexer https://wallet-testnet.catecash.com:443
```
