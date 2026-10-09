# nautilus-sodex examples

The eighteen programs here run against the live SoDEX venue. They manage API keys, check signing,
read market data and account state, and place orders on testnet to settle contract details that
the venue's documentation leaves open. They are not unit tests: apart from the ones marked
read-only, every one of them changes state at the venue.

Each program's full background - why it exists, what it proves, which venue behaviors it measured -
lives in the `//!` comment at the top of its `.rs` file. This page only collects what each one
needs, what it changes, and the order to use them in. Where the two disagree the source is right;
correct this page in the same change.

## Running

From the repository root:

```text
cargo run -q -p nautilus-sodex --example <name>
```

`-q` silences cargo's own build output and leaves the program's output alone. Every input is a
`SODEX_*` environment variable; none of the programs take arguments. How private keys should reach
those variables is under [Supplying keys](#supplying-keys).

## Two kinds of private key

| Key                | Variable                                                    | Can do                                                                                                      | Keep it                                                         |
| ------------------ | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| Master wallet      | `SODEX_MASTER_PRIVATE_KEY`                                  | Owns the account and can authorize withdrawals. Signs only `addAPIKey`, `revokeAPIKey`, `approveBuilderFee` | Offline. Brought out to register or revoke a key, then put back |
| Registered API key | `SODEX_API_PRIVATE_KEY`, together with `SODEX_API_KEY_NAME` | Places, cancels and amends orders, sets leverage                                                            | With the process that trades                                    |

The master key is exported from the venue's web UI at Settings -> Export Email Wallet. Only three
programs need it: `register_api_key`, `revoke_api_key` and `probe_key_permissions`.

**A permission mask does not narrow an API key at this venue.** `register_api_key.rs` records the
measurement of 2026-09-14: a key registered with `TRADE` withheld placed an order anyway, because
the signature the venue verifies does not cover that field. A key therefore has exactly two
bounds - its expiry (`SODEX_KEY_TTL_HOURS`) and revocation (`revoke_api_key`) - and a key handed to
an unattended process should carry an expiry.

## Supplying keys

The programs read keys from the environment and nowhere else, so anything that sets
`SODEX_API_PRIVATE_KEY` or `SODEX_MASTER_PRIVATE_KEY` for one command works. Two ways are shown:
one that runs anywhere, and one that keeps the keys in the macOS Keychain.

### Any platform: one `env` invocation

```text
 env SODEX_API_PRIVATE_KEY=<key> SODEX_ACCOUNT_ID=<aid> cargo run -q -p nautilus-sodex --example verify_signing
```

With `env` rather than `export`, the key lives only for this command instead of staying in the
shell's environment for everything run afterwards and every child process it spawns. The leading
space keeps the line out of shell history under `HIST_IGNORE_SPACE` (zsh) or
`HISTCONTROL=ignorespace` (bash). The key is still typed or pasted in plain text each time, and
has to be kept somewhere between runs. The command examples on this page use this form.

### macOS: the Keychain

Stored in a keychain, a key is encrypted at rest, never appears on a command line, and never
reaches shell history or a dotfile. The command reads it at the moment it runs:

```text
SODEX_API_PRIVATE_KEY="$(security find-generic-password -s sodex-api-testnet -a perps-key-01 -w)" \
SODEX_API_KEY_NAME=perps-key-01 \
SODEX_ACCOUNT_ID=<aid> \
SODEX_MARKET=perps \
cargo run -q -p nautilus-sodex --example verify_signing
```

This uses the shell's own `NAME=value command` prefix rather than `env`. The scope is the same -
the variables exist only for this command - but `env` would receive the expanded key as one of
its arguments, and a process's arguments are visible to anyone listing processes for as long as it
runs. The prefix form places the key straight into the command's environment. No leading space is
needed, since the line holds no secret.

The service name (`-s`) and account (`-a`) are only lookup labels. The convention used here is one
service per network, `sodex-api-testnet` and `sodex-api-mainnet`, with the registered key name as
the account. One key name can then be stored for both networks, and a key registered on both
engines (see [register_api_key](#register_api_key)) is stored once.

**API keys** go in the login keychain, which is unlocked while you are logged in:

```text
security add-generic-password -s sodex-api-testnet -a perps-key-01 -w
security find-generic-password -s sodex-api-testnet -a perps-key-01
security delete-generic-password -s sodex-api-testnet -a perps-key-01
```

The first stores a key. `-w` is deliberately last and given no value, so `security` prompts for
the key instead of taking it as an argument; paste the key `register_api_key` printed. Add `-U` to
overwrite an existing entry. The second confirms an entry exists without printing the key; the
third removes it, which belongs with every revocation.

**The master key** gets a keychain of its own, with its own password, locked except for the one
command that needs it. It authorizes withdrawals, so it should not sit in a keychain that is open
for the whole login session:

```text
security create-keychain ~/Library/Keychains/sodex-master.keychain
security set-keychain-settings -l -u -t 300 ~/Library/Keychains/sodex-master.keychain
printf 'master key: '; read -rs key; echo
printf 'add-generic-password -s sodex-master -a testnet -w %s %s\n' "$key" ~/Library/Keychains/sodex-master.keychain | security -i
unset key
security lock-keychain ~/Library/Keychains/sodex-master.keychain
```

`create-keychain` asks for the new keychain's password at the terminal; use one that differs from
the login password. `set-keychain-settings` makes it lock again after 300 idle seconds and when the
Mac sleeps, in case the explicit lock below is ever skipped. Created by path like this, the
keychain stays out of the default search list, so nothing that searches keychains generally finds
it, and every command has to name it.

Storing the key takes three lines rather than the one used for API keys, because
`add-generic-password` cannot both prompt and target a keychain: it prompts only when `-w` is the
last argument, and a named keychain must come after every option. Written the API-key way,
**`-w` takes the keychain's path as the key, and the entry lands in the login keychain.** So the
key is read without echo into a shell variable, and the command reaches `security` on its standard
input through `security -i`. `read` and `printf` are shell builtins in zsh and bash, so the key
never becomes any process's argument, and `unset` clears the variable afterwards.

A command that needs the master key then unlocks, reads, runs, and locks:

```text
security unlock-keychain ~/Library/Keychains/sodex-master.keychain && \
SODEX_MASTER_PRIVATE_KEY="$(security find-generic-password -s sodex-master -a testnet -w ~/Library/Keychains/sodex-master.keychain)" \
SODEX_ACCOUNT_ID=<aid> \
SODEX_API_KEY_NAME=perps-key-01 \
SODEX_MARKET=perps \
cargo run -q -p nautilus-sodex --example register_api_key; \
security lock-keychain ~/Library/Keychains/sodex-master.keychain
```

`unlock-keychain` prompts for that keychain's password. The final `;` rather than `&&` locks the
keychain whether or not the program succeeded.

Things this does not change:

- **A missing entry is an empty key, not a stopped command.** If the lookup finds nothing,
  `security` prints an error, the substitution yields an empty string, and the program still
  starts. It then refuses the key as having length 0 before sending anything, so nothing reaches
  the venue - but read the first lines of output rather than assuming the lookup worked.
- **Any of your processes can read an API key while the login keychain is unlocked.** An entry
  created by `security` trusts `security` itself, so the lookup above never asks for confirmation,
  and neither would the same lookup run by anything else under your account. Adding `-T ""` when
  storing removes that trust, and macOS then asks for confirmation on every read - stronger, and
  unusable for an unattended process. The separate keychain is what protects the master key.
- **The running program still holds the key in its environment.** The Keychain protects the key at
  rest and keeps it off the command line; it does nothing for a process that is already running.

## Common variables

| Variable                | Values                | Meaning                                                                                                            |
| ----------------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `SODEX_NETWORK`         | `testnet` / `mainnet` | **Every program defaults to testnet**; only the exact string `mainnet` selects mainnet, so a typo lands on testnet |
| `SODEX_MARKET`          | `spot` / `perps`      | Selects the engine. **The default differs between programs**; see the Engine column of the overview                |
| `SODEX_ACCOUNT_ID`      | integer               | The account's `aid`, from `/accounts/{address}/state`                                                              |
| `SODEX_WALLET_ADDRESS`  | `0x...`               | The master wallet address, used for unsigned account and key-list reads                                            |
| `SODEX_API_KEY_NAME`    | string                | The name the key was registered under. Signed requests are matched by name and key together                        |
| `SODEX_API_PRIVATE_KEY` | hex                   | Printed once at registration and not recoverable afterwards                                                        |
| `SODEX_SYMBOL_ID`       | integer               | The venue's numeric symbol id. `1` is `vBTC_vUSDC` on spot and `BTC-USD` on perps                                  |

### Finding the account id

The `aid` comes from the account state read, which is unsigned and needs only the wallet address:

```text
curl -s https://testnet-gw.sodex.dev/api/v1/spot/accounts/<0x...>/state | jq '.data.aid'
```

Use `mainnet-gw` for mainnet. Either engine works, `spot` or `perps` in the path: both answer the
same `aid` for one wallet, so there is one id per wallet per network, not one per engine. The
response also carries `uid`, which has matched `aid` so far; the programs need `aid`.

**Spot and perps keep separate key sets**, even under one account id. A key registered on perps is
unknown to spot, and a spot request signed with it comes back `API key not found`. That message
reads like a credential problem and is usually an engine mix-up; run `list_api_keys` to see which
engine the key actually lives on.

## Overview

"Changes state" says whether the program changes anything at the venue. "Engine" is what runs when
`SODEX_MARKET` is unset; "fixed" means the program hardcodes the engine and ignores the variable.

| Example                 | Purpose                                                               | Credentials                          | Changes state                                               | Engine       |
| ----------------------- | --------------------------------------------------------------------- | ------------------------------------ | ----------------------------------------------------------- | ------------ |
| `register_api_key`      | Registers a new API key, or an existing one on the other engine       | master                               | Yes: adds a key                                             | perps        |
| `revoke_api_key`        | Revokes an API key and measures that it stopped working               | master, ideally plus the revoked key | Yes: removes a key                                          | perps        |
| `list_api_keys`         | Lists the keys each engine holds                                      | none                                 | No                                                          | both         |
| `probe_key_permissions` | Asks whether the venue accepts a trade-but-not-withdraw mask          | master                               | Yes: registers a throwaway key if accepted, then revokes it | perps        |
| `verify_signing`        | Checks with a no-op that the venue accepts this key's signatures      | API key                              | Clears a pending dead-man scheduled cancel, if any          | perps        |
| `fetch_bars`            | Fetches historical klines                                             | none                                 | No                                                          | spot         |
| `stream_market_data`    | Streams candle, trade and ticker pushes                               | none                                 | No                                                          | spot         |
| `fetch_reports`         | Builds reconciliation reports from the live account                   | none                                 | No                                                          | spot         |
| `probe_channels`        | Asks which WebSocket channels exist                                   | none                                 | No                                                          | spot         |
| `probe_account_stream`  | Recovers the `accountUpdate` subscription fields from type errors     | none                                 | No                                                          | spot         |
| `probe_rest_endpoints`  | Asks which REST paths exist, with unsigned requests                   | none                                 | No                                                          | spot         |
| `place_and_cancel`      | Rests a limit order far from the market and cancels it                | API key                              | Yes: real order                                             | spot, fixed  |
| `place_modify_cancel`   | Rests an order, amends it, reads it back, cancels it                  | API key                              | Yes: real order                                             | perps        |
| `probe_limit_quantity`  | Isolates why perps limit orders were refused as `quantity is invalid` | API key                              | Yes: real order, cancelled                                  | perps, fixed |
| `cancel_open_orders`    | Cancels every resting order on one engine and reads back to confirm   | API key                              | Yes: cancels                                                | spot         |
| `observe_fill`          | Produces one real spot fill to read the fill wire shape               | API key                              | Yes: market buy, then sell                                  | spot, fixed  |
| `observe_position`      | Opens one real perps position to read the position wire shape         | API key                              | Yes: opens, then closes                                     | perps, fixed |
| `set_leverage`          | Sets leverage and margin mode, optionally moves margin                | API key                              | Yes: affects every later position                           | perps, fixed |

The order-placing programs are meant for testnet. They accept `SODEX_NETWORK=mainnet` like the
rest, and then the orders are real.

## Key lifecycle

The order is: register, check the list, check signing, use, revoke, check the list again. The
first time on a new account, pull the whole revocation once on a throwaway key, so the emergency
brake is known to work before any key goes to an unattended process.

### register_api_key

```text
 env SODEX_MASTER_PRIVATE_KEY=<master key> \
     SODEX_ACCOUNT_ID=<aid> \
     SODEX_API_KEY_NAME=perps-key-01 \
     SODEX_MARKET=perps \
     SODEX_KEY_TTL_HOURS=720 \
     cargo run -q -p nautilus-sodex --example register_api_key
```

- `SODEX_MASTER_PRIVATE_KEY` and `SODEX_ACCOUNT_ID` are required. `SODEX_API_KEY_NAME` defaults to
  `api-key-01`.
- Without `SODEX_KEY_TTL_HOURS` the key never expires, and the program says so.
- Without `SODEX_API_PRIVATE_KEY` a new key is generated and its private half is **printed once and
  written nowhere**. Store it before the terminal scrolls away - on macOS with the
  `add-generic-password` line under [Supplying keys](#supplying-keys) - since losing it means
  revoking the key and registering another.
- **One key for both engines:** register on one engine as above, then run again with
  `SODEX_MARKET` switched and `SODEX_API_PRIVATE_KEY` set to the key just printed. The second run
  registers that address rather than generating a new keypair.
- The venue's mainnet UI also creates keys (More -> API, at `/apikeys`), capped at five and valid
  for 1 to 180 days. The testnet UI has no such page.

### list_api_keys

```text
SODEX_WALLET_ADDRESS=<0x...> cargo run -q -p nautilus-sodex --example list_api_keys
```

Holds no key at all, so it still works after the last key is revoked. A registration, revocation
or expiry is confirmed when this list agrees, not when the venue acknowledges the request. Both
engines are listed and the raw JSON is printed alongside the parsed rows: the parsed form has no
permission field, so only the raw response shows whether the venue reports one.

Unset, `SODEX_WALLET_ADDRESS` falls back to the test wallet hardcoded in the source. Set it to read
any other account.

### verify_signing

```text
 env SODEX_API_KEY_NAME=perps-key-01 \
     SODEX_API_PRIVATE_KEY=<API key> \
     SODEX_ACCOUNT_ID=<aid> \
     SODEX_MARKET=perps \
     SODEX_WALLET_ADDRESS=<0x...> \
     cargo run -q -p nautilus-sodex --example verify_signing
```

Sends `scheduleCancel` with no timestamp. That only clears a pending dead-man schedule and touches
no order or position, yet it exercises the whole trading-domain signing path. The venue publishes
no expected signature bytes, so only a live request shows that a signature will be accepted.

The optional `SODEX_WALLET_ADDRESS` makes it check the unsigned key list first: is a key of this
name registered on this engine, and under this key's address? A wrong name, a wrong engine and a
wrong key all come back from the venue as the same `API key not found`; the list says which.

### revoke_api_key

```text
 env SODEX_MASTER_PRIVATE_KEY=<master key> \
     SODEX_ACCOUNT_ID=<aid> \
     SODEX_API_KEY_NAME=throwaway-key-01 \
     SODEX_API_PRIVATE_KEY=<the key being revoked> \
     SODEX_MARKET=perps \
     cargo run -q -p nautilus-sodex --example revoke_api_key
```

- `SODEX_MASTER_PRIVATE_KEY`, `SODEX_ACCOUNT_ID` and `SODEX_API_KEY_NAME` are required.
- **Supply `SODEX_API_PRIVATE_KEY` as well.** With it the program measures the revocation: a no-op
  signed by the key must be accepted before, the master wallet revokes, and the same no-op must be
  refused after - retried up to 6 times, 1 second apart, while the venue's key set settles. If the
  key is already refused before, the program stops without revoking, since a refusal afterwards
  would then prove nothing.
- Without it the key is still revoked, but the only evidence is the venue's acknowledgement, which
  says the request was accepted, not that the key is dead.
- If the key is still honored after the 6 attempts the program exits non-zero. Treat the key as
  live and revoke it through the venue's own interface.
- **Revoking a key a running node is using kills it mid-flight.** Every order, cancel and amend
  that node signs starts failing.

### probe_key_permissions

```text
 env SODEX_MASTER_PRIVATE_KEY=<master key> \
     SODEX_ACCOUNT_ID=<aid> \
     SODEX_MARKET=perps \
     cargo run -q -p nautilus-sodex --example probe_key_permissions
```

A one-off research tool, not part of routine operation. In three steps - control, mirror, under
test - it asks whether the venue accepts a mask that allows trading and denies withdrawals. If the
venue accepts, a throwaway key is really registered (its private half is never printed), used for
one order to see whether the mask binds, and revoked immediately. Optional: `SODEX_SYMBOL_ID`
(default `1`). The measured answer is under "Two kinds of private key" above.

## Read-only

None of these needs a key or changes state.

| Example                | Command                                                                               | Optional (default)                                                                                                                                                                                         |
| ---------------------- | ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fetch_bars`           | `cargo run -q -p nautilus-sodex --example fetch_bars`                                 | `SODEX_SYMBOL` (`vBTC_vUSDC`), `SODEX_INTERVAL` (`1h`), `SODEX_START_MINS` (unset: a fixed count of bars)                                                                                                  |
| `stream_market_data`   | `cargo run -q -p nautilus-sodex --example stream_market_data`                         | `SODEX_SYMBOL` (`vBTC_vUSDC`), `SODEX_INTERVAL` (`1m`), `SODEX_SECONDS` (`90`)                                                                                                                             |
| `fetch_reports`        | `SODEX_WALLET_ADDRESS=<0x...> cargo run -q -p nautilus-sodex --example fetch_reports` | `SODEX_WALLET_ADDRESS` (the hardcoded test wallet)                                                                                                                                                         |
| `probe_channels`       | `cargo run -q -p nautilus-sodex --example probe_channels`                             | `SODEX_CHANNEL` (sweep one channel's parameter shapes), `SODEX_LISTEN` (comma-separated channels, subscribed together and printed interleaved), `SODEX_SECONDS` (`10`), `SODEX_ACCOUNT_ID`, `SODEX_SYMBOL` |
| `probe_account_stream` | `cargo run -q -p nautilus-sodex --example probe_account_stream`                       | `SODEX_PHASE=subsets` (second phase: enumerate field subsets), `SODEX_CHANNEL` (`accountUpdate`), `SODEX_SECONDS` (`45`), `SODEX_ACCOUNT_ID`, `SODEX_SYMBOL`, `SODEX_COIN` (`vUSDC`)                       |
| `probe_rest_endpoints` | `cargo run -q -p nautilus-sodex --example probe_rest_endpoints`                       | `SODEX_ADDRESS` (the hardcoded test wallet), `SODEX_ACCOUNT_ID`                                                                                                                                            |

All of them also take `SODEX_NETWORK` and `SODEX_MARKET`. Trades are sparse on testnet; to see
`stream_market_data` print some, set `SODEX_NETWORK=mainnet`, which is still a public read.

`fetch_reports` cannot verify fills: an account that has never traded answers `[]`.
`probe_rest_endpoints` sends its `POST` and `DELETE` requests unsigned, so the venue rejects them
at the signature check before any business logic, and it deliberately probes no parameterless
bulk-mutation path such as `cancel-all`. Note that it reads the wallet from `SODEX_ADDRESS`, not
`SODEX_WALLET_ADDRESS` like the others.

## Orders and account settings

These need an API key and **change state at the venue**. The commands below leave `SODEX_NETWORK`
unset, which is testnet. `SODEX_API_KEY_NAME` defaults to `api-key-01` for spot and `perps-key-01`
for perps; set it whenever the registered name differs.

### cancel_open_orders

```text
 env SODEX_API_KEY_NAME=perps-key-01 \
     SODEX_API_PRIVATE_KEY=<key registered on that engine> \
     SODEX_ACCOUNT_ID=<aid> \
     SODEX_WALLET_ADDRESS=<0x...> \
     SODEX_MARKET=perps \
     cargo run -q -p nautilus-sodex --example cancel_open_orders
```

Cancels **everything** resting on the selected engine, not only what one program placed, and
leaves positions alone. It then reads the account again and exits non-zero if anything is still
resting. It exists because stopping the live node once left two orders resting on perps, so
cleanup must not depend on the node's shutdown working.

`SODEX_MARKET` defaults to spot, so cleaning up perps needs it set.

### place_and_cancel

```text
 env SODEX_API_KEY_NAME=api-key-01 \
     SODEX_API_PRIVATE_KEY=<key registered on spot> \
     SODEX_ACCOUNT_ID=<aid> \
     cargo run -q -p nautilus-sodex --example place_and_cancel
```

Optional: `SODEX_SYMBOL_ID` (`1`), `SODEX_LIMIT_PRICE` (`40000`), `SODEX_QUANTITY` (`0.001`). The
price sits far below the market so the order rests instead of filling; **do not raise it toward
the market**. `SODEX_LEAVE_RESTING=true` skips the cancel, leaving the order for
`cancel_open_orders` to exercise a multi-order cancel.

### place_modify_cancel

```text
 env SODEX_API_KEY_NAME=perps-key-01 \
     SODEX_API_PRIVATE_KEY=<key registered on perps> \
     SODEX_ACCOUNT_ID=<aid> \
     SODEX_WALLET_ADDRESS=<0x...> \
     cargo run -q -p nautilus-sodex --example place_modify_cancel
```

The amendment is verified by reading the account back, not by the acknowledgement. Optional:

| Variable                                                                            | Default                            | Effect                                                                                                        |
| ----------------------------------------------------------------------------------- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `SODEX_MARKET`                                                                      | `perps`                            | `spot` also requires `SODEX_CHANGE_VIA=replace`, since spot serves no modify route                            |
| `SODEX_CHANGE_VIA`                                                                  | modify                             | `replace` uses the replace route instead: served on both engines, with an id of its own                       |
| `SODEX_REPLACE_KEEPS_ID`                                                            | no                                 | `true` gives the replacement the client order id it replaces                                                  |
| `SODEX_MODIFY_BY`                                                                   | venue order id                     | `cl_ord_id` addresses the order by its client order id                                                        |
| `SODEX_MODIFY_FIELD`                                                                | price                              | `quantity` changes the quantity to `0.0003` instead                                                           |
| `SODEX_TIME_IN_FORCE`                                                               | `GTX` (post-only)                  | `gtc` rests a plain limit order. The venue has refused to amend post-only orders with `OrderCannotBeModified` |
| `SODEX_SYMBOL_ID` / `SODEX_LIMIT_PRICE` / `SODEX_MODIFIED_PRICE` / `SODEX_QUANTITY` | `1` / `70000` / `69000` / `0.0002` | Both prices sit far below the market so the order rests                                                       |

### probe_limit_quantity

```text
 env SODEX_API_KEY_NAME=perps-key-01 \
     SODEX_API_PRIVATE_KEY=<key registered on perps> \
     SODEX_ACCOUNT_ID=<aid> \
     SODEX_QUANTITY=0.0002 \
     cargo run -q -p nautilus-sodex --example probe_limit_quantity
```

A research tool separating the two explanations for `quantity is invalid`: a trailing zero in the
quantity string, or a constraint limit orders carry and market orders do not. Change only
`SODEX_QUANTITY` between runs. Optional: `SODEX_SYMBOL_ID` (`1`), `SODEX_LIMIT_PRICE` (`70000`). The
order rests far from the market and is cancelled immediately.

### observe_fill

```text
 env SODEX_API_KEY_NAME=api-key-01 \
     SODEX_API_PRIVATE_KEY=<key registered on spot> \
     SODEX_ACCOUNT_ID=<aid> \
     SODEX_WALLET_ADDRESS=<0x...> \
     cargo run -q -p nautilus-sodex --example observe_fill
```

**Places two real market orders**: a minimum-size buy, then a sell to flatten. The sell reads the
balance and sells what is held, rounded down to the step size, because the buy's fee is taken in
the base asset (ordering `0.001` vBTC credits `0.00099935`). The sell runs even when reading the
fill fails.

Optional: `SODEX_SYMBOL_ID` (`1`), `SODEX_QUANTITY` (`0.001`), `SODEX_BASE_COIN` (`vBTC`),
`SODEX_STEP_SIZE` (`0.00001`). `SODEX_FLATTEN_ONLY=1` skips the buy and only sells what is already
held, which cleans up a leftover from an earlier run.

### observe_position

```text
 env SODEX_API_KEY_NAME=perps-key-01 \
     SODEX_API_PRIVATE_KEY=<key registered on perps> \
     SODEX_ACCOUNT_ID=<aid> \
     SODEX_WALLET_ADDRESS=<0x...> \
     SODEX_SIDE=buy \
     cargo run -q -p nautilus-sodex --example observe_position
```

**Places two real market orders**: one to open, then a reduce-only one to close. The close runs
even when reading the position fails. Both `/positions` and `/state` are printed, because they
express a flat account differently. `SODEX_SIDE=sell` opens a short; run both sides, since a long
reports `size` unsigned and says nothing about how a short is expressed.

Optional: `SODEX_SYMBOL_ID` (`1`), `SODEX_QUANTITY` (`0.0002`). `SODEX_HOLD_ONLY=1` skips the close,
and whatever is left open then has to be flattened by hand.

### set_leverage

```text
 env SODEX_API_KEY_NAME=perps-key-01 \
     SODEX_API_PRIVATE_KEY=<key registered on perps> \
     SODEX_ACCOUNT_ID=<aid> \
     SODEX_SYMBOL_ID=1 \
     SODEX_LEVERAGE=20 \
     SODEX_MARGIN_MODE=cross \
     cargo run -q -p nautilus-sodex --example set_leverage
```

Perps only. Leverage applies to the instrument, so it affects every position opened afterwards,
and the venue may refuse a reduction while a position is open. `SODEX_LEVERAGE` defaults to `20`;
`SODEX_MARGIN_MODE` takes `cross` (default) or `isolated`.

The optional `SODEX_MARGIN_AMOUNT` also moves margin against the instrument's isolated position.
**Its sign convention is unobserved**: the SDK types it as a plain decimal and nothing says whether
a negative amount withdraws. Whoever runs it first should record what it does.
