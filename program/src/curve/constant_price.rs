// SPDX-License-Identifier: AGPL-3.0-only

//! Simple constant price swap curve, set at init
use {
    crate::{
        curve::{calculator::{
            map_zero_to_none, CurveCalculator, DynPack, RoundDirection, SwapWithoutFeesResult,
            TradeDirection, TradingTokenResult,
        }, redemption_rate::RAY},
        error::SwapError,
    },
    arrayref::{array_mut_ref, array_ref},
    solana_program::{
        program_error::ProgramError,
        program_pack::{IsInitialized, Pack, Sealed},
    },
    spl_math::{checked_ceil_div::CheckedCeilDiv, precise_number::PreciseNumber, uint::U256},
};

/// Get the amount of pool tokens for the given amount of token A or B.
///
/// The constant product implementation uses the Balancer formulas found at
/// <https://docs.balancer.fi/whitepaper.pdf>, specifically
/// in the case for 2 tokens, each weighted at 1/2.
pub fn trading_tokens_to_pool_tokens(
    token_b_price: u128,
    source_amount: u128,
    swap_token_a_amount: u128,
    swap_token_b_amount: u128,
    pool_supply: u128,
    trade_direction: TradeDirection,
    round_direction: RoundDirection,
) -> Option<u128> {
    let token_b_price = U256::from(token_b_price);
    let scaling_factor = U256::from(RAY);

    let given_value = match trade_direction {
        TradeDirection::AtoB => U256::from(source_amount),
        TradeDirection::BtoA => U256::from(source_amount)
            .checked_mul(token_b_price)?
            .checked_div(scaling_factor)?,
    };

    let total_value = U256::from(swap_token_b_amount)
        .checked_mul(token_b_price)?
        .checked_div(scaling_factor)?
        .checked_add(U256::from(swap_token_a_amount))?;

    let pool_supply = U256::from(pool_supply);

    match round_direction {
        RoundDirection::Floor => Some(
            pool_supply
                .checked_mul(given_value)?
                .checked_div(total_value)?
                .as_u128(),
        ),
        RoundDirection::Ceiling => Some(
            pool_supply
                .checked_mul(given_value)?
                .checked_ceil_div(total_value)?
                .0
                .as_u128(),
        ),
    }
}

/// ConstantPriceCurve struct implementing CurveCalculator
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConstantPriceCurve {
    /// Amount of token A required to get 1 token B
    /// equals to real price * RAY
    pub token_b_price: u128,
}

impl CurveCalculator for ConstantPriceCurve {
    /// Charge only full multiples of price; drop remainder.
    fn swap_without_fees(
        &self,
        source_amount: u128,
        _swap_source_amount: u128,
        _swap_destination_amount: u128,
        trade_direction: TradeDirection,
        _timestamp: Option<u128>
    ) -> Option<SwapWithoutFeesResult> {
        let token_b_price = U256::from(self.token_b_price);
        let source_amount = U256::from(source_amount);
        let scaling_factor = U256::from(RAY);

        let (source_amount_swapped, destination_amount_swapped) = match trade_direction {
            TradeDirection::BtoA => (source_amount, source_amount.checked_mul(token_b_price)?.checked_div(scaling_factor)?),
            TradeDirection::AtoB => {
                let destination_amount = source_amount
                    .checked_mul(scaling_factor)?
                    .checked_div(token_b_price)?;


                let (source_amount_used, _) = destination_amount
                    .checked_mul(token_b_price)?
                    .checked_ceil_div(scaling_factor)?;

                if source_amount_used > source_amount {
                    return None;
                }

                (source_amount_used, destination_amount)

                // let mut source_amount_swapped = source_amount;

                // // if there is a remainder from buying token B, floor
                // // token_a_amount to avoid taking too many tokens, but
                // // don't recalculate the fees
                // let remainder = source_amount_swapped.checked_rem(token_b_price)?;
                // if remainder > U256::zero() {
                //     source_amount_swapped = source_amount.checked_sub(remainder)?;
                // }

                // (source_amount_swapped, destination_amount_swapped)
            }
        };

        let source_amount_swapped = map_zero_to_none(source_amount_swapped.as_u128())?;
        let destination_amount_swapped = map_zero_to_none(destination_amount_swapped.as_u128())?;
        
        Some(SwapWithoutFeesResult {
            source_amount_swapped,
            destination_amount_swapped,
        })
    }

    /// Convert `pool_tokens` into the proportional amounts of each trading token,
    /// given current pool reserves and total pool-token supply.
    /// Rounds according to `round_direction`.
    fn pool_tokens_to_trading_tokens(
        &self,
        pool_tokens: u128,
        pool_token_supply: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        round_direction: RoundDirection,
    ) -> Option<TradingTokenResult> {
        let pool_tokens = U256::from(pool_tokens);
        let pool_token_supply = U256::from(pool_token_supply);
        let swap_token_a_amount = U256::from(swap_token_a_amount);
        let swap_token_b_amount = U256::from(swap_token_b_amount);

        let (token_a_amount, token_b_amount) = match round_direction {
            RoundDirection::Floor => (
                pool_tokens.checked_mul(swap_token_a_amount)?
                    .checked_div(pool_token_supply)?,
                pool_tokens.checked_mul(swap_token_b_amount)?
                    .checked_div(pool_token_supply)?,
            ),
            RoundDirection::Ceiling => {
                let (a, _) = pool_tokens.checked_mul(swap_token_a_amount)?
                    .checked_ceil_div(pool_token_supply)?;
                let (b, _) = pool_tokens.checked_mul(swap_token_b_amount)?
                    .checked_ceil_div(pool_token_supply)?;
                (a, b)
            }
        };

        Some(TradingTokenResult {
            token_a_amount: token_a_amount.as_u128(),
            token_b_amount: token_b_amount.as_u128()
        })
    }

    /// Get the amount of pool tokens for the given amount of token A and B
    /// For the constant price curve, the total value of the pool is weighted
    /// by the price of token B.
    fn deposit_single_token_type(
        &self,
        source_amount: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        pool_supply: u128,
        trade_direction: TradeDirection,
        _timestamp: Option<u128>,
    ) -> Option<u128> {
        trading_tokens_to_pool_tokens(
            self.token_b_price,
            source_amount,
            swap_token_a_amount,
            swap_token_b_amount,
            pool_supply,
            trade_direction,
            RoundDirection::Floor,
        )
    }

    fn withdraw_single_token_type_exact_out(
        &self,
        source_amount: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        pool_supply: u128,
        trade_direction: TradeDirection,
        round_direction: RoundDirection,
        _timestamp: Option<u128>,
    ) -> Option<u128> {
        trading_tokens_to_pool_tokens(
            self.token_b_price,
            source_amount,
            swap_token_a_amount,
            swap_token_b_amount,
            pool_supply,
            trade_direction,
            round_direction,
        )
    }

    fn validate(&self, _timestamp: Option<u128>) -> Result<(), SwapError> {
        if self.token_b_price == 0 {
            Err(SwapError::InvalidCurve)
        } else {
            Ok(())
        }
    }

    fn validate_supply(&self, token_a_amount: u64, _token_b_amount: u64) -> Result<(), SwapError> {
        if token_a_amount == 0 {
            return Err(SwapError::EmptySupply);
        }
        Ok(())
    }

    /// The total normalized value of the constant price curve adds the total
    /// value of the token B side to the token A side.
    ///
    /// Note that since most other curves use a multiplicative invariant, ie.
    /// `token_a * token_b`, whereas this one uses an addition,
    /// ie. `token_a + token_b`.
    ///
    /// At the end, we divide by 2 to normalize the value between the two token
    /// types.
    fn normalized_value(
        &self,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        _timestamp: Option<u128>
    ) -> Option<PreciseNumber> {
        let token_b_price = U256::from(self.token_b_price);
        let scaling_factor = U256::from(RAY);

        let swap_token_a_amount = U256::from(swap_token_a_amount);
        let swap_token_b_amount = U256::from(swap_token_b_amount);

        let swap_token_b_value = swap_token_b_amount
            .checked_mul(token_b_price)?
            .checked_div(scaling_factor)?;

        let value = swap_token_a_amount
            .checked_add(swap_token_b_value)?
            .checked_div(U256::from(2))?;

        PreciseNumber::new(value.try_into().ok()?)
    }
}

/// IsInitialized is required to use `Pack::pack` and `Pack::unpack`
impl IsInitialized for ConstantPriceCurve {
    fn is_initialized(&self) -> bool {
        true
    }
}
impl Sealed for ConstantPriceCurve {}
impl Pack for ConstantPriceCurve {
    const LEN: usize = 16;
    fn pack_into_slice(&self, output: &mut [u8]) {
        (self as &dyn DynPack).pack_into_slice(output);
    }

    fn unpack_from_slice(input: &[u8]) -> Result<ConstantPriceCurve, ProgramError> {
        let token_b_price = array_ref![input, 0, 16];
        Ok(Self {
            token_b_price: u128::from_le_bytes(*token_b_price),
        })
    }
}

impl DynPack for ConstantPriceCurve {
    fn pack_into_slice(&self, output: &mut [u8]) {
        let token_b_price = array_mut_ref![output, 0, 16];
        *token_b_price = self.token_b_price.to_le_bytes();
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::curve::calculator::{
            test::{
                check_curve_value_from_swap, check_deposit_token_conversion,
                check_withdraw_token_conversion, total_and_intermediate,
                CONVERSION_BASIS_POINTS_GUARANTEE,
            },
            INITIAL_SWAP_POOL_AMOUNT,
        },
        proptest::prelude::*,
    };

    const MAX_PRICE: u64 = (u128::MAX / RAY) as u64; 

    #[test]
    fn swap_calculation_no_price() {
        let swap_source_amount: u128 = 0;
        let swap_destination_amount: u128 = 0;
        let source_amount: u128 = 100;
        let token_b_price = 1;
        let scaled_token_b_price = token_b_price * RAY;
        let curve = ConstantPriceCurve { token_b_price: scaled_token_b_price };

        let expected_result = SwapWithoutFeesResult {
            source_amount_swapped: source_amount,
            destination_amount_swapped: source_amount,
        };

        let result = curve
            .swap_without_fees(
                source_amount,
                swap_source_amount,
                swap_destination_amount,
                TradeDirection::AtoB,
                None
            )
            .unwrap();
        assert_eq!(result, expected_result);

        let result = curve
            .swap_without_fees(
                source_amount,
                swap_source_amount,
                swap_destination_amount,
                TradeDirection::BtoA,
                None
            )
            .unwrap();
        assert_eq!(result, expected_result);
    }

    #[test]
    fn pack_flat_curve() {
        let token_b_price = 1_251_258;
        let scaled_token_b_price = token_b_price * RAY;
        let curve = ConstantPriceCurve { token_b_price: scaled_token_b_price };

        let mut packed = [0u8; ConstantPriceCurve::LEN];
        Pack::pack_into_slice(&curve, &mut packed[..]);
        let unpacked = ConstantPriceCurve::unpack(&packed).unwrap();
        assert_eq!(curve, unpacked);

        let mut packed = vec![];
        packed.extend_from_slice(&scaled_token_b_price.to_le_bytes());
        let unpacked = ConstantPriceCurve::unpack(&packed).unwrap();
        assert_eq!(curve, unpacked);
    }

    #[test]
    fn swap_calculation_large_price() {
        let token_b_price = 1_123_513;
        let scaled_token_price = token_b_price * RAY;

        let curve = ConstantPriceCurve {
            token_b_price: scaled_token_price,
        };

        let token_b_amount = 500u128;
        let token_a_amount = token_b_amount * token_b_price;

        let bad_result = curve.swap_without_fees(
            token_b_price - 1u128,
            token_a_amount,
            token_b_amount,
            TradeDirection::AtoB,
            None
        );

        assert!(bad_result.is_none());

        let bad_result = curve.swap_without_fees(
            1u128, 
            token_a_amount, 
            token_b_amount, 
            TradeDirection::AtoB, None
        );

        assert!(bad_result.is_none());

        let result = curve
            .swap_without_fees(
                token_b_price,
                token_a_amount,
                token_b_amount,
                TradeDirection::AtoB,
                None
            )
            .unwrap();

        assert_eq!(result.source_amount_swapped, token_b_price);
        assert_eq!(result.destination_amount_swapped, 1u128);
    }

    #[test]
    fn swap_calculation_max_min() {
        let token_b_price = MAX_PRICE as u128;
        let scaled_token_b_price = token_b_price * RAY;

        let curve = ConstantPriceCurve {
            token_b_price: scaled_token_b_price,
        };
        let token_b_amount = 1u128;
        let token_a_amount = token_b_price;
        let bad_result = curve.swap_without_fees(
            token_b_price - 1u128,
            token_a_amount,
            token_b_amount,
            TradeDirection::AtoB,
            None
        );

        assert!(bad_result.is_none());

        let bad_result = curve.swap_without_fees(
            1u128, 
            token_a_amount, 
            token_b_amount, 
            TradeDirection::AtoB, None
        );

        assert!(bad_result.is_none());

        let bad_result = curve.swap_without_fees(
            0u128, 
            token_a_amount, 
            token_b_amount, 
            TradeDirection::AtoB, None
        );

        assert!(bad_result.is_none());

        let result = curve.swap_without_fees(
            token_b_price,
            token_a_amount,
            token_b_amount,
            TradeDirection::AtoB,
            None
        )
        .unwrap();

        assert_eq!(result.source_amount_swapped, token_b_price);
        assert_eq!(result.destination_amount_swapped, 1u128);
    }

    proptest! {
        #[test]
        fn deposit_token_conversion_a_to_b(
            // in the pool token conversion calcs, we simulate trading half of
            // source_token_amount, so this needs to be at least 2
            source_token_amount in 2..u64::MAX,
            swap_source_amount in 1..u64::MAX,
            swap_destination_amount in 1..u64::MAX,
            pool_supply in INITIAL_SWAP_POOL_AMOUNT..u64::MAX as u128,
            token_b_price in 1..MAX_PRICE,
        ) {
            let scaled_token_b_price = (token_b_price as u128) * RAY;

            let traded_source_amount = source_token_amount / 2;
            // Make sure that the trade yields at least 1 token B
            prop_assume!(traded_source_amount / token_b_price >= 1);
            // Make sure there's enough tokens to get back on the other side
            prop_assume!(traded_source_amount / token_b_price <= swap_destination_amount);

            let curve = ConstantPriceCurve {
                token_b_price: scaled_token_b_price,
            };

            check_deposit_token_conversion(
                &curve,
                source_token_amount as u128,
                swap_source_amount as u128,
                swap_destination_amount as u128,
                TradeDirection::AtoB,
                pool_supply,
                CONVERSION_BASIS_POINTS_GUARANTEE,
                None
            );
        }
    }

    proptest! {
        #[test]
        fn deposit_token_conversion_b_to_a(
            // in the pool token conversion calcs, we simulate trading half of
            // source_token_amount, so this needs to be at least 2
            source_token_amount in 2..u32::MAX, // kept small to avoid proptest rejections
            swap_source_amount in 1..u64::MAX,
            swap_destination_amount in 1..u64::MAX,
            pool_supply in INITIAL_SWAP_POOL_AMOUNT..u64::MAX as u128,
            token_b_price in 1..u32::MAX, // kept small to avoid proptest rejections
        ) {
            let scaled_token_b_price = (token_b_price as u128) * RAY;

            let curve = ConstantPriceCurve {
                token_b_price: scaled_token_b_price,
            };
            let token_b_price = token_b_price as u128;
            let source_token_amount = source_token_amount as u128;
            let swap_source_amount = swap_source_amount as u128;
            let swap_destination_amount = swap_destination_amount as u128;
            // The constant price curve needs to have enough destination amount
            // on the other side to complete the swap
            prop_assume!(token_b_price * source_token_amount / 2 <= swap_destination_amount);

            check_deposit_token_conversion(
                &curve,
                source_token_amount,
                swap_source_amount,
                swap_destination_amount,
                TradeDirection::BtoA,
                pool_supply,
                CONVERSION_BASIS_POINTS_GUARANTEE,
                None
            );
        }
    }

    proptest! {
        #[test]
        fn withdraw_token_conversion(
            (pool_token_supply, pool_token_amount) in total_and_intermediate(u64::MAX),
            swap_token_a_amount in 1..u64::MAX,
            swap_token_b_amount in 1..u32::MAX, // kept small to avoid proptest rejections
            token_b_price in 1..u32::MAX, // kept small to avoid proptest rejections
        ) {
            let scaled_token_b_price = (token_b_price as u128) * RAY;

            let curve = ConstantPriceCurve {
                token_b_price: scaled_token_b_price,
            };
            let token_b_price = token_b_price as u128;
            let pool_token_amount = pool_token_amount as u128;
            let pool_token_supply = pool_token_supply as u128;
            let swap_token_a_amount = swap_token_a_amount as u128;
            let swap_token_b_amount = swap_token_b_amount as u128;

            let value = curve.normalized_value(swap_token_a_amount, swap_token_b_amount, None).unwrap();

            // Make sure we trade at least one of each token
            prop_assume!(pool_token_amount * value.to_imprecise().unwrap() >= 2 * token_b_price * pool_token_supply);

            let withdraw_result = curve
                .pool_tokens_to_trading_tokens(
                    pool_token_amount,
                    pool_token_supply,
                    swap_token_a_amount,
                    swap_token_b_amount,
                    RoundDirection::Floor,
                )
                .unwrap();
            prop_assume!(withdraw_result.token_a_amount <= swap_token_a_amount);
            prop_assume!(withdraw_result.token_b_amount <= swap_token_b_amount);

            check_withdraw_token_conversion(
                &curve,
                pool_token_amount,
                pool_token_supply,
                swap_token_a_amount,
                swap_token_b_amount,
                TradeDirection::AtoB,
                // TODO see why this needs to be so high
                CONVERSION_BASIS_POINTS_GUARANTEE * 5,
                None
            );
            check_withdraw_token_conversion(
                &curve,
                pool_token_amount,
                pool_token_supply,
                swap_token_a_amount,
                swap_token_b_amount,
                TradeDirection::BtoA,
                // TODO see why this needs to be so high
                CONVERSION_BASIS_POINTS_GUARANTEE * 5,
                None
            );
        }
    }

    proptest! {
        #[test]
        fn curve_value_does_not_decrease_from_swap_a_to_b(
            source_token_amount in 1..u64::MAX,
            swap_source_amount in 1..u64::MAX,
            swap_destination_amount in 1..u64::MAX,
            token_b_price in 1..MAX_PRICE,
        ) {
            let scaled_token_b_price = (token_b_price as u128) * RAY;
            // Make sure that the trade yields at least 1 token B
            prop_assume!(source_token_amount / token_b_price >= 1);
            // Make sure there's enough tokens to get back on the other side
            prop_assume!(source_token_amount / token_b_price <= swap_destination_amount);
            let curve = ConstantPriceCurve { token_b_price: scaled_token_b_price };
            check_curve_value_from_swap(
                &curve,
                source_token_amount as u128,
                swap_source_amount as u128,
                swap_destination_amount as u128,
                TradeDirection::AtoB,
                None
            );
        }
    }

    proptest! {
        #[test]
        fn curve_value_does_not_decrease_from_swap_b_to_a(
            source_token_amount in 1..u32::MAX, // kept small to avoid proptest rejections
            swap_source_amount in 1..u64::MAX,
            swap_destination_amount in 1..u64::MAX,
            token_b_price in 1..u32::MAX, // kept small to avoid proptest rejections
        ) {
            // The constant price curve needs to have enough destination amount
            // on the other side to complete the swap
            let scaled_token_b_price = (token_b_price as u128) * RAY;
            let curve = ConstantPriceCurve { token_b_price: scaled_token_b_price };
            let token_b_price = token_b_price as u128;
            let source_token_amount = source_token_amount as u128;
            let swap_destination_amount = swap_destination_amount as u128;
            let swap_source_amount = swap_source_amount as u128;
            // The constant price curve needs to have enough destination amount
            // on the other side to complete the swap
            prop_assume!(token_b_price * source_token_amount <= swap_destination_amount);
            check_curve_value_from_swap(
                &curve,
                source_token_amount,
                swap_source_amount,
                swap_destination_amount,
                TradeDirection::BtoA,
                None
            );
        }
    }

    proptest! {
        #[test]
        fn curve_value_does_not_decrease_from_deposit(
            pool_token_amount in 2..u64::MAX, // minimum 2 to splitting on deposit
            pool_token_supply in INITIAL_SWAP_POOL_AMOUNT..u64::MAX as u128,
            swap_token_a_amount in 1..u64::MAX,
            swap_token_b_amount in 1..u32::MAX, // kept small to avoid proptest rejections
            token_b_price in 1..u32::MAX, // kept small to avoid proptest rejections
        ) {
            let scaled_token_b_price = (token_b_price as u128) * RAY;
            let curve = ConstantPriceCurve { token_b_price: scaled_token_b_price };
            let pool_token_amount = pool_token_amount as u128;
            let swap_token_a_amount = swap_token_a_amount as u128;
            let swap_token_b_amount = swap_token_b_amount as u128;
            let token_b_price = token_b_price as u128;

            let value = curve.normalized_value(swap_token_a_amount, swap_token_b_amount, None).unwrap();

            // Make sure we trade at least one of each token
            prop_assume!(pool_token_amount * value.to_imprecise().unwrap() >= 2 * token_b_price * pool_token_supply);
            let deposit_result = curve
                .pool_tokens_to_trading_tokens(
                    pool_token_amount,
                    pool_token_supply,
                    swap_token_a_amount,
                    swap_token_b_amount,
                    RoundDirection::Ceiling,
                )
                .unwrap();
            let new_swap_token_a_amount = swap_token_a_amount + deposit_result.token_a_amount;
            let new_swap_token_b_amount = swap_token_b_amount + deposit_result.token_b_amount;
            let new_pool_token_supply = pool_token_supply + pool_token_amount;

            let new_value = curve.normalized_value(new_swap_token_a_amount, new_swap_token_b_amount, None).unwrap();

            // the following inequality must hold:
            // new_value / new_pool_token_supply >= value / pool_token_supply
            // which reduces to:
            // new_value * pool_token_supply >= value * new_pool_token_supply

            let pool_token_supply = PreciseNumber::new(pool_token_supply).unwrap();
            let new_pool_token_supply = PreciseNumber::new(new_pool_token_supply).unwrap();
            //let value = U256::from(value);
            //let new_value = U256::from(new_value);

            assert!(new_value.checked_mul(&pool_token_supply).unwrap().greater_than_or_equal(&value.checked_mul(&new_pool_token_supply).unwrap()));
        }
    }

    proptest! {
        #[test]
        fn curve_value_does_not_decrease_from_withdraw(
            (pool_token_supply, pool_token_amount) in total_and_intermediate(u64::MAX),
            swap_token_a_amount in 1..u64::MAX,
            swap_token_b_amount in 1..u32::MAX, // kept small to avoid proptest rejections
            token_b_price in 1..u32::MAX, // kept small to avoid proptest rejections
        ) {
            let scaled_token_b_price = (token_b_price as u128) * RAY;
            let curve = ConstantPriceCurve { token_b_price: scaled_token_b_price };
            let pool_token_amount = pool_token_amount as u128;
            let pool_token_supply = pool_token_supply as u128;
            let swap_token_a_amount = swap_token_a_amount as u128;
            let swap_token_b_amount = swap_token_b_amount as u128;
            let token_b_price = token_b_price as u128;

            let value = curve.normalized_value(swap_token_a_amount, swap_token_b_amount, None).unwrap();

            // Make sure we trade at least one of each token
            prop_assume!(pool_token_amount * value.to_imprecise().unwrap() >= 2 * token_b_price * pool_token_supply);
            prop_assume!(pool_token_amount <= pool_token_supply);
            let withdraw_result = curve
                .pool_tokens_to_trading_tokens(
                    pool_token_amount,
                    pool_token_supply,
                    swap_token_a_amount,
                    swap_token_b_amount,
                    RoundDirection::Floor,
                )
                .unwrap();
            prop_assume!(withdraw_result.token_a_amount <= swap_token_a_amount);
            prop_assume!(withdraw_result.token_b_amount <= swap_token_b_amount);
            let new_swap_token_a_amount = swap_token_a_amount - withdraw_result.token_a_amount;
            let new_swap_token_b_amount = swap_token_b_amount - withdraw_result.token_b_amount;
            let new_pool_token_supply = pool_token_supply - pool_token_amount;

            let new_value = curve.normalized_value(new_swap_token_a_amount, new_swap_token_b_amount, None).unwrap();

            // the following inequality must hold:
            // new_value / new_pool_token_supply >= value / pool_token_supply
            // which reduces to:
            // new_value * pool_token_supply >= value * new_pool_token_supply

            let pool_token_supply = PreciseNumber::new(pool_token_supply).unwrap();
            let new_pool_token_supply = PreciseNumber::new(new_pool_token_supply).unwrap();
            assert!(new_value.checked_mul(&pool_token_supply).unwrap().greater_than_or_equal(&value.checked_mul(&new_pool_token_supply).unwrap()));
        }
    }
}
