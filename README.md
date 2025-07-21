# Nova PSM

This repository contains the implementation of a PSM (Peg Stability Module) for the Solana Virtual Machine. The functionality within this program is based largely on Spark's PSM [link](https://github.com/sparkdotfi/spark-psm).

The SPL Token Swap functionality facilitates swapping between any two SPL Tokens, which can be facilitated through a choice of curves. `ConstantProduct`, `ConstantPrice` and `Offset` curves are supported in the original implementation, but are not anticipated to be required initially. 

This implementation extends the range of curves offered to include a new `RedemptionRate` curve type, modeled on the functionality within [PSM3.sol](https://github.com/sparkdotfi/spark-psm/blob/master/src/PSM3.sol) and the corresponding `RateProvider` implementation for EVM, but adapted for the nuissances of Solana and the SVM. As such, the this program also includes equivalent functionality associated with the  `RateProvider` contract in Spark's EVM implementation.

The `RedemptionRate` curve is intended to support the conversion of Solana USDS to Solana sUSDS (Savings USDS) and subsequent redemption, although it could be used to support similar functionality for any pair of tokens that follows a similar approach and can provide similarly described redemption rate configurations.

The choice to build on-top of the SPL Token Swap program was based on an intention to leverage broad ecosystem adoption and composability. The SPL Token Swap Program has been integrated with by many protocols, searchers, algorithmic traders and crucially aggregators. By leveraging this basis and ensuring that key instruction interfaces remain unchanged, this program remains entirely compatible for this audience of possible integrators.

All liquidity provision functionality and core swapping functionality remains unchanged and inherited from the original SPL Token Swap program. The SPL Token Swap program has been audited many times and has been widely used and battle-tested across Solana DeFi.

## Instruction Overview

**`Initialize`** - Initializes a new token swap pool with specified fees and swap curve parameters. This is the first instruction that must be called to set up a new liquidity pool.

**`Swap`** - Executes a token swap between two tokens in the pool. Users specify the input amount and minimum output amount to prevent excessive slippage. The swap follows the pool's pricing curve and applies configured fees.

**`DepositAllTokenTypes`** - Allows users to deposit both token types into the pool in the current ratio. In return, users receive pool tokens representing their share of the liquidity pool. Users can specify maximum amounts for each token to prevent excessive slippage.

**`WithdrawAllTokenTypes`** - Enables users to withdraw both token types from the pool by burning their pool tokens. The withdrawal amount is based on the user's share of the pool and the current token ratio. Users can specify minimum amounts for each token to prevent excessive slippage.

**`DepositSingleTokenTypeExactAmountIn`** - Allows users to deposit a single token type into the pool. The input amount is specified exactly, and the output pool tokens are calculated based on the current exchange rate. Users can specify a minimum amount of pool tokens to receive.

**`WithdrawSingleTokenTypeExactAmountOut`** - Enables users to withdraw a specific amount of a single token type from the pool. Users specify the exact output amount desired and the maximum pool tokens they're willing to burn.

**`SetRates`** - Updates the redemption rate curve parameters (ssr, rho, chi) for the pool. This instruction requires appropriate permissions to execute.

**`InitializePermission`** - Creates a new permission account with specified authority and capabilities. This is used to manage who can perform administrative actions on the pool.

**`UpdatePermission`** - Modifies the permissions of an existing permission account. This can update whether an account has super admin privileges or can update curve parameters.


## `RedemptionRate` Curve Explanation

The redemption rate model is intended to provide a facility to enable an up-to-date conversion rate to be calculated at the time of request (i.e. at the current block's timestamp), without the need for the rate to be continuously posted. Provided the underlying rate of accrual has not changed, this will replicate (with a very high degree of precision) the rate that would be reflected at the same time on the source protocol implementing the same model.

The functionality implemented here is based on Spark's implementation [SSROracleBase.sol](https://github.dev/sparkdotfi/xchain-ssr-oracle/blob/master/src/SSROracleBase.sol).

The configuration consists of `ssr`, `chi` and `rho` parameters — which together allow the prevailing redemption rate to be calculated for the current block timestamp, without needing the current redemption rate to be continuously reported. 

NOTE: The complexity of the calculation increases with the time over which the rate must compound (specifically the time between `rho` and now). Testing shows that periods of up to 3650 days can be calculated within a `swap` instruction at under 400,000 compute units. When developing infrastructure to provide updates to the configuration, this should be considered in determining a suitable minimum frequency of update.

NOTE: When a rate change occurred (i.e. change in `ssr` parameter), calculated rates will be slightly misaligned from those in the original protocol. For typical rates (0-20% APY) the change in rate over short periods of time is minimal, and so the attack vector is very limited over short periods of time. However, over time this divergence will grow, potentially creating a risk of loss for liquidity providers. When developing infrastructure to provide updates to the configuration, this should be considered in order to minimize the time between rates occurring on the source/original protocol and being reflected within this implementation's configuration.
 
### Updating the `RedemptionRate` configuration

For swap pairs configured to use the `RedemptionRate` curve type, the underlying price or swap rate determined is affected by the configuration within the curve's state. These parameters are intended to be updated over time to reflect changes in the redemption rate from the core issuing protocol of the yield-bearing token (e.g. Sky's core protocol on Ethereum for sUSDS/USDS creation and redemption).

This rate is updated through a permissioned instruction (`SetRates`). The swap authority remains the super-authority for a particular swap pair, but other update authority can optionally be added using `InitializePermission` and managed using `UpdatePermission`.

NOTE: Naturally the need to update this rate is a mission critical one, and both (a) updates of invalid rate configurations; as well as (b) failure to provide rate updates for extended period; pose risk a risk of loss a liquidity providers. This should be well-understood and suitably addressed when building infrastructure to carry-out this process and in the operations and management of the authority keys associated with this process.

### RedemptionRate Curve Unit Tests

The RedemptionRate curve includes comprehensive unit tests covering:

- **Power function (`_rpow`) tests:**
  - Overflow protection with large bases and exponents
  - Identity cases (`x^0 = 1`, `0^0 = 1`, `0^n = 0`)
  - Integer powers with property-based testing
  - Fractional base computations with accuracy verification
  - Floating-point comparison tests for non-integer results
  - Interest rate calculations (5% APY, 100% APY over various time periods)
  - Rounding behavior verification
  - Small and large exponent edge cases

- **Rate setting validation tests:**
  - `rho` (timestamp) boundary conditions (future timestamps, decreasing values)
  - `ssr` boundary conditions (below RAY, above max_ssr)
  - `chi` growth rate validation (decreasing chi, excessive growth rates)
  - `max_ssr` enforcement when configured
  - No-limit behavior when `max_ssr` is unset

- **Swap precision tests:**
  - sUSDS to USDS conversion accuracy across different amounts
  - USDS to sUSDS conversion with slippage verification
  - Scaled testing up to 100 million tokens

- **Basic swap calculations:**
  - No-price scenarios (1:1 conversion)
  - Large price differentials
  - Maximum/minimum value edge cases

- **Serialization:**
  - Pack/unpack round-trip verification
  - Binary format consistency

- **Property-based testing:**
  - Deposit token conversions (A to B and B to A) across wide parameter ranges
  - Withdraw token conversions with relaxed tolerance (0.5% vs 0.1% for other curves)
  - Curve value preservation during swaps and liquidity operations
  - Invariant maintenance across deposits and withdrawals

- **Test coverage notes:**
  - Withdraw tests use 100 basis points (1.0%) instead of the standard 20 basis points (0.2%)
  used in `ConstantPrice` to accommodate the compounding calculations inherent in the RedemptionRate model

For more information on tests implemented on other curve types, please see the SPL Token Swap program [link](https://github.com/solana-labs/solana-program-library/tree/master/token-swap) (the original implementation on which this program was based)



### Integration tests

You can test the JavaScript bindings and on-chain interactions using
`solana-test-validator`, included in the Solana Tool Suite.  See the
[CLI installation instructions](https://docs.solana.com/cli/install-solana-cli-tools).

From `./js`, install the required modules:

```sh
npm i
```

Then run all tests:

```sh
npm run test
```

#### Modifications from forked Integration Tests

- **Instruction format changes:**
  - Added 8 extra bytes to instruction data for future referral code support
  - Tests verify backwards compatibility and that critical instructions remain functional

- **Curve type adaptation:**
  - Modified all tests to use `RedemptionRate` curve instead of `ConstantProduct`
  - Tests cover complete lifecycle: swap creation, deposits, withdrawals, and swaps

- **Simplified fee structure:**
  - All fees set to 0 (trading fees, owner fees, host fees)
  - Maintains original test scenarios while focusing on core redemption rate functionality


## Building master

To build a development version of the Token Swap program, you can use the normal
build command for Solana programs:

```sh
cargo build-sbf
```

## Testing

### Unit tests

Run unit tests from `./program/` using:

```sh
cargo test-sbf
```

## Deployment

### Prerequisites

- Solana CLI tools installed ([CLI installation instructions](https://docs.solana.com/cli/install-solana-cli-tools))
- SOL for rent and transaction fees

### Mainnet Deployment

```sh
# Set Solana CLI to mainnet
solana config set --url https://api.mainnet-beta.solana.com

# Ensure sufficient SOL
solana balance

# Deploy with production keypair
cargo build-sbf  
solana program deploy target/deploy/nova_psm.so --keypair ~/production-deployer.json
```

## Program Addresses

### SPL Swap

| Network | Program ID | Notes |
|---------|-----------|-------|
| **Mainnet** | `5B9vCSSga3qXgHca5Liy3WAQqC2HaB3sBsyjfkH47uYv` | Production deployment |
| **Devnet** | `5B9vCSSga3qXgHca5Liy3WAQqC2HaB3sBsyjfkH47uYv` | Testing environment |

### SPL Token Swap References

| Program | Network | Address |
|---------|---------|---------|
| SPL Token Swap | Mainnet | `SwapsVeCiPHMUAtzQWZw7RjsKjgCjhwU55QGu4U1Szw` |

### Key Token Addresses Solana

| Token | Network | Mint Address |
|-------|---------|--------------|
| **USDS** | Mainnet | `USDSwr9ApdHk5bvJKMjzff41FfuX8bSxdKcR81vTwcA` | 
| **sUSDS** | Mainnet | `TBD` |

### Key Token Addresses Ethereum

| Token | Network | Mint Address |
|-------|---------|--------------|
| **USDS** | Ethereum | `0xdC035D45d973E3EC169d2276DDab16f1e407384F` | 
| **sUSDS** | Mainnet | `0xa3931d71877C0E7a3148CB7Eb4463524FEc27fbD` |


## License
This project is licensed under the GNU Affero General Public License v3.0 
see the [LICENSE.txt](LICENSE.txt) file for details.


