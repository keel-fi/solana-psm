# Nova PSM

A PSM (Peg Stability Module) implementation for Solana, based on the 
SPL Token Swap program [link](https://github.com/solana-labs/solana-program-library/tree/master/token-swap).

Facilitates the conversion and redemption of Solana bridged sUSDS for USDS (and vice versa) 
using a redemption rate model that mirrors the approach used by Spark on remote chains [link](https://github.com/sparkdotfi/spark-psm/blob/master/src/PSM3.sol).

Extends the SPL Token Swap implementation with a new `RedemptionRate` curve type, alongside the existing `ConstantProduct` and `ConstantPrice` `Offset` curves.

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
cargo test
```

#### RedemptionRate Curve Unit Tests

The RedemptionRate curve includes comprehensive unit tests covering:

- **Power function (`_rpow`) tests:**
  - Overflow protection with large bases and exponents
  - Identity cases (x^0 = 1, 0^0 = 1, 0^n = 0)
  - Integer powers with property-based testing
  - Fractional base computations with accuracy verification
  - Floating-point comparison tests for non-integer results
  - Interest rate calculations (5% APY, 100% APY over various time periods)
  - Rounding behavior verification
  - Small and large exponent edge cases

- **Rate setting validation tests:**
  - Rho (timestamp) boundary conditions (future timestamps, decreasing values)
  - SSR boundary conditions (below RAY, above max_ssr)
  - Chi growth rate validation (decreasing chi, excessive growth rates)
  - Max SSR enforcement when configured
  - No-limit behavior when max_ssr is unset

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


