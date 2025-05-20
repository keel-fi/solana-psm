// SPDX-License-Identifier: AGPL-3.0-only

//! Curve inspired by Spark PSM3
use arrayref::array_ref;
use solana_program::{
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack, Sealed},
};
use spl_math::{
    checked_ceil_div::CheckedCeilDiv, precise_number::PreciseNumber, uint::U256
};
use crate::error::SwapError;
use shank::ShankType;

use super::calculator::{
    map_zero_to_none, 
    CurveCalculator, 
    DynPack, 
    RoundDirection, 
    SwapWithoutFeesResult, 
    TradeDirection, TradingTokenResult
};

/// Get the amount of pool tokens for the given amount of token A or B.
pub fn trading_tokens_to_pool_tokens(
    token_b_price: U256,
    ray: U256,
    source_amount: u128,
    swap_token_a_amount: u128,
    swap_token_b_amount: u128,
    pool_supply: u128,
    trade_direction: TradeDirection,
    round_direction: RoundDirection,
) -> Option<u128> {
    let given_value = match trade_direction {
        TradeDirection::AtoB => U256::from(source_amount),
        TradeDirection::BtoA => U256::from(source_amount)
            .checked_mul(token_b_price)?
            .checked_div(ray)?
    };

    let total_value = U256::from(swap_token_b_amount)
        .checked_mul(token_b_price)?
        .checked_div(ray)?
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

/// RedemptionRateCurve struct implementing CurveCalculator
#[derive(Clone, Debug, Default, PartialEq, ShankType)]
pub struct RedemptionRateCurve {
    /// Fixed-point scaling factor.
    pub ray: u128,
    /// Maximum allowed SSR value.
    pub max_ssr: u128,
    /// Current Stable Savings Rate (SSR), compounding per second, scaled by `ray`.
    pub ssr: u128,
    /// Timestamp (in seconds) of the last update to `chi`
    pub rho: u128,
    /// Accumulated conversion factor at timestamp `rho`, scaled by `ray`.
    pub chi: u128,
}

impl RedemptionRateCurve {
    /// Returns conversion rate
    pub fn get_conversion_rate(
        &self,
        timestamp: u128
    ) -> Option<U256> {
        if timestamp == self.rho {
            return Some(U256::from(self.chi))
        } 
        let duration = timestamp.checked_sub(self.rho)?;
        let rate = self._rpow(self.ssr, duration)? * U256::from(self.chi) / U256::from(self.ray);
        Some(rate)
    }

    /// Custom pow function
    /// Reference implementation:
    /// https://github.com/sparkdotfi/xchain-ssr-oracle/blob/0593279e643285bd4d54e23e37a050e0cad215ce/src/SSROracleBase.sol#L123-L146
    pub fn _rpow(
        &self,
        x: u128,
        n: u128,
    ) -> Option<U256> {
        let mut z: U256;
        let x_u256 = U256::from(x);
        let n_u256 = U256::from(n);
        let ray_u256 = U256::from(self.ray);

        if x_u256 == U256::zero() {
            if n_u256 == U256::zero() {
                z = ray_u256;
            } else {
                z = U256::zero();
            }
        } else {
            let half = ray_u256 / U256::from(2);
            let n_mod_2 = n_u256 % U256::from(2);
            if n_mod_2 == U256::zero() {
                z = ray_u256;
            } else {
                z = x_u256;
            }
            
            let mut n = n_u256 / U256::from(2);
            let mut x = x_u256;
            
            while n > U256::zero() {
                // Calculate x^2
                let xx = x * x;
                // Check for overflow
                if xx / x != x {
                    return None;
                }
                // Add half for rounding
                let xx_round = xx + half;
                if xx_round < xx {
                    return None;
                }
                // Divide by RAY
                x = xx_round / ray_u256;
                
                // If n is odd, multiply by x
                if n % U256::from(2) == U256::one() {
                    let zx = z * x;
                    // Check for overflow
                    if x != U256::zero() && zx / x != z {
                        return None;
                    }
                    // Add half for rounding
                    let zx_round = zx + half;
                    if zx_round < zx {
                        return None;
                    }
                    // Divide by RAY
                    z = zx_round / ray_u256;
                }
                
                n = n / U256::from(2);
            }
        }
        Some(z)
    }

    /// Set new rates and returns a new RedemptionRateCurve
    pub fn set_rates(
        &self,
        ssr: u128,
        rho: u128,
        chi: u128,
        current_timestamp: u128,
    ) -> Result<RedemptionRateCurve, ProgramError> {
        if rho > current_timestamp {
            return Err(SwapError::InvalidRho.into())
        }
        if ssr < self.ray {
            return Err(SwapError::InvalidSsr.into())
        }
        if self.max_ssr != 0 && ssr > self.max_ssr {
            return Err(SwapError::InvalidSsr.into())
        }

        let new_calculator = if self.rho == 0 {
            RedemptionRateCurve {
                ray: self.ray,
                max_ssr: self.max_ssr,
                ssr,
                rho,
                chi
            }
        } else {
            if rho < self.rho {
                return Err(SwapError::InvalidRho.into())
            }
            if chi < self.chi {
                return Err(SwapError::InvalidChi.into())
            }
            if self.max_ssr != 0 {
                let duration = rho
                    .checked_sub(self.rho)
                    .ok_or(ProgramError::ArithmeticOverflow)?;
                
                let chi_max = self._rpow(self.max_ssr, duration)
                    .ok_or(SwapError::CalculationFailure)?
                    .checked_mul(U256::from(self.chi))
                    .ok_or(ProgramError::ArithmeticOverflow)?
                    .checked_div(U256::from(self.ray))
                    .ok_or(ProgramError::ArithmeticOverflow)?;
                
    
                if U256::from(chi) > chi_max {
                    return Err(SwapError::InvalidChi.into())
                }
            }
    
            RedemptionRateCurve {
                ray: self.ray,
                max_ssr: self.max_ssr,
                ssr,
                rho,
                chi
            }
        };

        Ok(new_calculator)
    }
}

impl CurveCalculator for RedemptionRateCurve {
    fn swap_without_fees(
        &self,
        source_amount: u128,
        _swap_source_amount: u128,
        _swap_destination_amount: u128,
        trade_direction: TradeDirection,
        timestamp: Option<u128>
    ) -> Option<SwapWithoutFeesResult> {
        let token_b_price = self.get_conversion_rate(timestamp?)?;
        let source_amount = U256::from(source_amount);
        let ray = U256::from(self.ray);

        let (source_amount_swapped, destination_amount_swapped) = match trade_direction {
            TradeDirection::BtoA => (source_amount, source_amount.checked_mul(token_b_price)?.checked_div(ray)?),
            TradeDirection::AtoB => {
                let destination_amount = source_amount
                    .checked_mul(ray)?
                    .checked_div(token_b_price)?;

                let (source_amount_used, _) = destination_amount
                    .checked_mul(token_b_price)?
                    .checked_ceil_div(ray)?;
                

                if source_amount_used > source_amount {
                    return None;
                }
            
                (source_amount_used, destination_amount)
            }
        };

        let source_amount_swapped = map_zero_to_none(source_amount_swapped.as_u128())?;
        let destination_amount_swapped = map_zero_to_none(destination_amount_swapped.as_u128())?;

        Some(SwapWithoutFeesResult { 
            source_amount_swapped, 
            destination_amount_swapped 
        })
    }

    fn pool_tokens_to_trading_tokens(
        &self,
        pool_tokens: u128,
        pool_token_supply: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        round_direction: super::calculator::RoundDirection,
        timestamp: Option<u128>
    ) -> Option<super::calculator::TradingTokenResult> {

        let token_b_price = self.get_conversion_rate(timestamp?)?;
        let ray = U256::from(self.ray);

        let pool_tokens = U256::from(pool_tokens);
        let pool_token_supply = U256::from(pool_token_supply);

        let total_value = U256::from(self
            .normalized_value(swap_token_a_amount, swap_token_b_amount, timestamp)?
            .to_imprecise()?);

        let (token_a_amount, token_b_amount) = match round_direction {
            RoundDirection::Floor => {

                let token_a_amount = pool_tokens
                    .checked_mul(total_value)?
                    .checked_div(pool_token_supply)?
                    .min(U256::from(swap_token_a_amount));

                let token_b_amount = pool_tokens
                    .checked_mul(total_value)?
                    .checked_mul(ray)?
                    .checked_div(token_b_price)?
                    .checked_div(pool_token_supply)?
                    .min(U256::from(swap_token_b_amount)); 

                (token_a_amount, token_b_amount)
            }
            RoundDirection::Ceiling => {
                let (token_a_amount, _) = pool_tokens
                    .checked_mul(total_value)?
                    .checked_ceil_div(pool_token_supply)?;

                let (pool_value_as_token_b, _) = pool_tokens
                    .checked_mul(total_value)?
                    .checked_mul(ray)?
                    .checked_ceil_div(token_b_price)?;

                let (token_b_amount, _) =
                    pool_value_as_token_b.checked_ceil_div(pool_token_supply)?;

                (token_a_amount, token_b_amount)
            }
        };
        Some(TradingTokenResult {
            token_a_amount: token_a_amount.as_u128(),
            token_b_amount: token_b_amount.as_u128(),
        })
    }

    fn deposit_single_token_type(
        &self,
        source_amount: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        pool_supply: u128,
        trade_direction: TradeDirection,
        timestamp: Option<u128>,
    ) -> Option<u128> {
        let token_b_price = self.get_conversion_rate(timestamp?)?;
        let ray = U256::from(self.ray);

        trading_tokens_to_pool_tokens(
            token_b_price, 
            ray, 
            source_amount, 
            swap_token_a_amount, 
            swap_token_b_amount, 
            pool_supply, 
            trade_direction, 
            RoundDirection::Floor
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
        timestamp: Option<u128>,
    ) -> Option<u128> {

        let token_b_price = self.get_conversion_rate(timestamp?)?;
        let ray = U256::from(self.ray);

        trading_tokens_to_pool_tokens(
            token_b_price, 
            ray, 
            source_amount, 
            swap_token_a_amount, 
            swap_token_b_amount, 
            pool_supply, 
            trade_direction, 
            round_direction
        )
    }

    fn validate(&self, timestamp: Option<u128>) -> Result<(), SwapError> {
        let timestamp = timestamp
            .ok_or(SwapError::MissingTimestamp)?;

        let token_b_price = self.get_conversion_rate(timestamp)
            .ok_or(SwapError::CalculationFailure)?;

        if token_b_price == U256::zero() {
            Err(SwapError::InvalidCurve)
        } else {
            Ok(())
        }
    }

    fn validate_supply(
        &self, 
        token_a_amount: u64, 
        _token_b_amount: u64
    ) -> Result<(), SwapError> {
        if token_a_amount == 0 {
            return Err(SwapError::EmptySupply);
        }
        Ok(())
    }

    fn normalized_value(
        &self,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        timestamp: Option<u128>
    ) -> Option<spl_math::precise_number::PreciseNumber> {
        let token_b_price = self.get_conversion_rate(timestamp?)?;
        let ray = U256::from(self.ray);
        let swap_token_b_amount = U256::from(swap_token_b_amount);

        let swap_token_b_value = swap_token_b_amount
            .checked_mul(token_b_price)?
            .checked_div(ray)?;

        // special logic in case we're close to the limits, avoid overflowing u128
        let value = if swap_token_b_value.saturating_sub(U256::from(u64::MAX))
            > U256::MAX.saturating_sub(U256::from(u64::MAX))
        {
            swap_token_b_value
                .checked_div(U256::from(2))?
                .checked_add(U256::from(swap_token_a_amount).checked_div(U256::from(2))?)?
        } else {
            U256::from(swap_token_a_amount)
                .checked_add(swap_token_b_value)?
                .checked_div(U256::from(2))?
        };
    
        PreciseNumber::new(value.try_into().ok()?)
    }

}

impl IsInitialized for RedemptionRateCurve {
    fn is_initialized(&self) -> bool {
        true
    }
}

impl Sealed for RedemptionRateCurve {}

impl Pack for RedemptionRateCurve {
    const LEN: usize = 80;

    fn pack_into_slice(&self, output: &mut [u8]) {
        (self as &dyn DynPack).pack_into_slice(output);
    }

    fn unpack_from_slice(input: &[u8]) -> Result<RedemptionRateCurve, ProgramError> {
        let ray = array_ref![input, 0, 16];
        let max_ssr = array_ref![input, 16, 16];
        let ssr = array_ref![input, 32, 16];
        let rho = array_ref![input, 48, 16];
        let chi = array_ref![input, 64, 16];

        Ok(Self {
            ray: u128::from_le_bytes(*ray),
            max_ssr: u128::from_le_bytes(*max_ssr),
            ssr: u128::from_le_bytes(*ssr),
            rho: u128::from_le_bytes(*rho),
            chi: u128::from_le_bytes(*chi),
        })
    }
}

impl DynPack for RedemptionRateCurve {
    fn pack_into_slice(&self, output: &mut [u8]) {
        let (ray, rest) = output.split_at_mut(16);
        let (max_ssr, rest) = rest.split_at_mut(16);
        let (ssr, rest) = rest.split_at_mut(16);
        let (rho, rest) = rest.split_at_mut(16);
        let (chi, _) = rest.split_at_mut(16);

        ray.copy_from_slice(&self.ray.to_le_bytes());
        max_ssr.copy_from_slice(&self.max_ssr.to_le_bytes());
        ssr.copy_from_slice(&self.ssr.to_le_bytes());
        rho.copy_from_slice(&self.rho.to_le_bytes());
        chi.copy_from_slice(&self.chi.to_le_bytes());
    }
}

#[cfg(test)]
mod rpow_tests {
    use super::*;
    use proptest::prelude::*;

    const RAY: u128 = 10u128.pow(27);
    const FIVE_PCT_APY_SSR: u128 = 1_000_000_001_547_125_957_863_212_448;
    const SECONDS_PER_YEAR: u128 = 365 * 24 * 60 * 60;
    const SECONDS_PER_FIFTY_YEARS: u128 = 365 * 24 * 60 * 60 * 50;
    const ONE_HUNDRED_PCT_APY_SSR: u128 = 1_000_000_021_979_553_151_239_153_020;

    fn create_test_curve(
        ssr: u128,
        rho: u128,
        chi: u128,
        max_ssr: u128
    ) -> RedemptionRateCurve {
        RedemptionRateCurve {
            ray: RAY, 
            max_ssr,
            ssr,
            rho,
            chi,
        }
    }

    #[test]
    fn test_rpow_overflow_protection() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            0
        );
        
        // Test with very large base and exponent -- 100% APY for 10 years
        let large_base =  ONE_HUNDRED_PCT_APY_SSR;
        let large_exp = SECONDS_PER_YEAR * 10;
        
        // This should not overflow due to U256 usage
        let result = curve._rpow(large_base, large_exp).unwrap();
        assert!(result > U256::zero());
    }


    // tolerance_pct is in percentage (1.0 means 1%)
    fn assert_close_to_float(actual: U256, expected_float: f64, tolerance_pct: f64) {
        // convert expected float to U256 scaled by RAY
        let expected = (expected_float * RAY as f64) as u128;
        let expected_u256 = U256::from(expected);
        
        // calculate allowable difference (tolerance_pct% of expected value)
        // Scale by 1000 to preserve precision for small percentages
        let tolerance_scaled = (tolerance_pct * 1000.0) as u128;
        let tolerance = (expected_u256 * U256::from(tolerance_scaled)) / U256::from(100000u128);
        
        // calculate actual difference
        let diff = if actual > expected_u256 {
            actual - expected_u256
        } else {
            expected_u256 - actual
        };
        
        assert!(
            diff <= tolerance,
            "values not close enough: actual {:?}, expected {:?}, diff {:?}, tolerance {:?} ({}%)",
            actual, expected_u256, diff, tolerance, tolerance_pct
        );
    }

    #[test]
    fn test_rpow_identity_cases() {
        let curve = create_test_curve(0, 0, 0, 0);
        
        // x^0 = RAY (1.0) for any x > 0
        assert_eq!(curve._rpow(RAY, 0).unwrap(), U256::from(RAY));
        assert_eq!(curve._rpow(2 * RAY, 0).unwrap(), U256::from(RAY));
        
        // 0^0 = RAY (1.0) by definition
        assert_eq!(curve._rpow(0, 0).unwrap(), U256::from(RAY));
        
        // 0^n = 0 for n > 0
        assert_eq!(curve._rpow(0, 1).unwrap(), U256::zero());
        assert_eq!(curve._rpow(0, 100).unwrap(), U256::zero());
        
        // x^1 = x for any x
        assert_eq!(curve._rpow(RAY, 1).unwrap(), U256::from(RAY));
        assert_eq!(curve._rpow(2 * RAY, 1).unwrap(), U256::from(2 * RAY));
    }

    proptest! {
        #[test]
        fn test_rpow_integer_powers(
            // test bases from 1 to 20
            base_multiplier in 1u32..21u32,
            // test exponents from 1 to 10
            exponent in 1u32..11u32,
        ) {
            let curve = create_test_curve(0, 0, 0, 0);
            
            // calculate base value (scaled by RAY)
            let base = RAY * base_multiplier as u128;
            
            // use the curve's _rpow function to calculate result
            let rpow_result = curve._rpow(base, exponent as u128).unwrap();
            
            // calculate expected result using Rust's native pow function
            // (base_multiplier^exponent) * RAY
            let expected_multiplier = (base_multiplier as u128).pow(exponent);
            
            // ensure we don't overflow u128
            prop_assume!(expected_multiplier <= u128::MAX / RAY);
            
            let expected = expected_multiplier * RAY;
            
            // assert that the _rpow result matches the expected result
            assert_eq!(
                rpow_result, 
                U256::from(expected),
                "incorrect integer power: {}^{} should be {} * RAY",
                base_multiplier, exponent, expected_multiplier
            );
        }
    }

    proptest! {
        #[test]
        fn test_rpow_fractional_base(
            // test denominators from 2 to 20 (representing fractions from 1/2 to 1/20)
            denominator in 2u32..21u32,
            // test exponents from 1 to 5
            exponent in 1u32..6u32,
        ) {
            let curve = create_test_curve(0, 0, 0, 0);
            
            // calculate base (RAY / denominator)
            let base = RAY / denominator as u128;
            
            // use _rpow to calculate base^exponent
            let result = curve._rpow(base, exponent as u128).unwrap();
            
            // for fraction 1/n, (1/n)^e = 1/(n^e)
            let denom_power = (denominator as u128).pow(exponent);
            let expected = RAY / denom_power;
            
            // allow for a small difference due to fixed-point rounding
            let diff = if result > U256::from(expected) {
                result - U256::from(expected)
            } else {
                U256::from(expected) - result
            };
            
            prop_assert!(
                diff <= U256::from(1),
                "fractional power too inaccurate: (1/{})^{} calculated as {}, expected {}, diff {}",
                denominator, exponent, result, expected, diff
            );
        }
    }

    #[test]
    fn test_rpow_specific_fractional_base_cases() {
        let curve = create_test_curve(0, 0, 0, 0);
                
        // 0.5^2 = 0.25
        let base = RAY / 2;
        let expected = RAY / 4;
        assert_eq!(curve._rpow(base, 2).unwrap(), U256::from(expected));
        
        // 0.5^3 = 0.125
        let expected = RAY / 8;
        assert_eq!(curve._rpow(base, 3).unwrap(), U256::from(expected));
        
        // 0.1^2 = 0.01
        let base = RAY / 10;
        let expected = RAY / 100;
        assert_eq!(curve._rpow(base, 2).unwrap(), U256::from(expected));
        
        // 0.25^4 = 0.00390625
        let base = RAY / 4;
        let expected = RAY / 256;
        assert_eq!(curve._rpow(base, 4).unwrap(), U256::from(expected));
        
        // 0.2^3 = 0.008
        let base = RAY / 5;
        let expected = RAY / 125;
        assert_eq!(curve._rpow(base, 3).unwrap(), U256::from(expected));
    }

    #[test]
    fn test_rpow_against_floating_point() {
        let curve = create_test_curve(0, 0, 0, 0);
        
        // 1.5^2 = 2.25
        let base = RAY + (RAY / 2);
        let result = curve._rpow(base, 2).unwrap();
        assert_close_to_float(result, 2.25, 1.0); // 1% tolerance
        
        // 1.1^10 ≈ 2.5937...
        let base = RAY + (RAY / 10);
        let result = curve._rpow(base, 10).unwrap();
        assert_close_to_float(result, 2.5937424601, 1.0); // 1% tolerance
        // 0.9^5 ≈ 0.59049
        let base = RAY - (RAY / 10);
        let result = curve._rpow(base, 5).unwrap();
        assert_close_to_float(result, 0.59049, 1.0); // 1% tolerance
    }

    #[test]
    fn test_rpow_interest_rates() {
        let curve = create_test_curve(0, 0, 0, 0);
        
        // 5% for 1 year should be close to 1.05
        let result = curve._rpow(FIVE_PCT_APY_SSR, SECONDS_PER_YEAR).unwrap();
        assert_close_to_float(result, 1.05, 0.001);
        
        // 5% for 2 years should be close to 1.1025 (1.05^2)
        let result = curve._rpow(FIVE_PCT_APY_SSR, 2 * SECONDS_PER_YEAR).unwrap();
        assert_close_to_float(result, 1.1025, 0.001);
        
        
        // 100% for 1 year should be close to 2.0
        let result = curve._rpow(ONE_HUNDRED_PCT_APY_SSR, SECONDS_PER_YEAR).unwrap();
        assert_close_to_float(result, 2.0, 0.001);

        // 5% APY for 50 years
        // Expected unscaled value: (1.05)^50 ≈ 11.467396597107005
        let result = curve._rpow(FIVE_PCT_APY_SSR, SECONDS_PER_FIFTY_YEARS).unwrap();
        assert_close_to_float(result, 11.467396597107005, 0.01);

        // 5% APY for 100 years
        // Expected unscaled value: (1.05)^100 ≈ 131.5012578490916
        let result = curve._rpow(FIVE_PCT_APY_SSR, SECONDS_PER_FIFTY_YEARS * 2).unwrap();
        assert_close_to_float(result, 131.5012578490916, 0.1);
    }


    #[test]
    fn test_rpow_rounding_behavior() {
        let curve = create_test_curve(0, 0, 0, 0);
        
        // test with 1.5^2 = 2.25 (no rounding needed)
        let base = RAY + (RAY / 2);  // 1.5 * RAY
        let expected = RAY * 9 / 4;  // 2.25 * RAY
        let result = curve._rpow(base, 2).unwrap();
        assert_eq!(
            result, 
            U256::from(expected), 
            "perfect square should not need rounding"
        );
        
        // test with 1.1^3 = 1.331 (with precise hard-coded value)
        let base = RAY + (RAY / 10);  // 1.1 * RAY
        // hard-coded expected value: 1.331 * RAY
        // 1.331 can be represented as 1331/1000
        let expected_cube = U256::from(RAY) * U256::from(1331) / U256::from(1000);
        let rpow_result = curve._rpow(base, 3).unwrap();
        
        assert_eq!(
            rpow_result, 
            expected_cube,
            "result should match precisely calculated value"
        );
        
        // test odd vs even exponent with small base
        let base_small = RAY + 1;  // just above 1.0
        
        // hard-coded expected values
        let expected_odd = U256::from(RAY) + U256::from(3);  // 1.000...003
        let expected_even = U256::from(RAY) + U256::from(4); // 1.000...004
        
        let result_odd = curve._rpow(base_small, 3).unwrap();
        let result_even = curve._rpow(base_small, 4).unwrap();
        
        assert_eq!(result_odd, expected_odd, "odd exponent result should match expected");
        assert_eq!(result_even, expected_even, "even exponent result should match expected");
        assert!(result_even > result_odd, "higher exponent should yield larger result");
    }

    #[test]
    fn test_rpow_small_exponents() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            0
        );
        
        // Test with small exponents (2, 3, 4)
        let base = RAY + 1; // Slightly above RAY
        let exp2 = curve._rpow(base, 2).unwrap();
        let exp3 = curve._rpow(base, 3).unwrap();
        let exp4 = curve._rpow(base, 4).unwrap();
        
        // Verify exponential growth
        assert!(exp3 > exp2);
        assert!(exp4 > exp3);
        
        // Verify the values are reasonable
        assert!(exp2 < U256::from(base) * U256::from(2));
        assert!(exp3 < U256::from(base) * U256::from(3));
        assert!(exp4 < U256::from(base) * U256::from(4));
    }

    #[test]
    fn test_rpow_large_exponents() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            0
        );
        
        // Test with large exponents (1 year, 2 years)
        let exp1y = curve._rpow(FIVE_PCT_APY_SSR, SECONDS_PER_YEAR).unwrap();
        let exp2y = curve._rpow(FIVE_PCT_APY_SSR, 2 * SECONDS_PER_YEAR).unwrap();
        
        // Verify exponential growth
        assert!(exp2y > exp1y);
        
        // Verify the values are reasonable (5% APY over 1 year)
        let expected_min = U256::from(RAY) + (U256::from(RAY) / U256::from(20)); // 5% growth

        // Check that exp1y is within 100_000_000_000 of expected_min
        let diff = if exp1y > expected_min {
            exp1y - expected_min
        } else {
            expected_min - exp1y
        };
        assert!(diff <= U256::from(100_000_000_000u128), "Difference too large: {:?}", diff);
    }


}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::curve::calculator::{
            test::{
                check_curve_value_from_swap, 
                check_deposit_token_conversion, 
                check_withdraw_token_conversion, 
                total_and_intermediate, 
                CONVERSION_BASIS_POINTS_GUARANTEE
            },
            INITIAL_SWAP_POOL_AMOUNT,
        },
        proptest::prelude::*,
    };

    const RAY: u128 = 10u128.pow(27);
    const FIVE_PCT_APY_SSR: u128 = 1_000_000_001_547_125_957_863_212_448;
    const ONE_HUNDRED_PCT_APY_SSR: u128 = 1_000_000_021_979_553_151_239_153_020;
    const SECONDS_PER_YEAR: u128 = 365 * 24 * 60 * 60;

    // Initial timestamp after skipping 1 year
    const INITIAL_TIMESTAMP: u128 = SECONDS_PER_YEAR;
    // Timestamp after skipping another year
    const SECOND_TIMESTAMP: u128 = 2 * SECONDS_PER_YEAR;

    fn create_test_curve(
        ssr: u128,
        rho: u128,
        chi: u128,
        max_ssr: u128
    ) -> RedemptionRateCurve {
        RedemptionRateCurve {
            ray: RAY, 
            max_ssr,
            ssr,
            rho,
            chi,
        }
    }

    proptest! {
        #[test]
        fn test_susds_usds_precision_slippage_scaled(
            multiplier in 1u64..100_000_000u64, // up to 100,000,000 sUSDS
        ) {
            let ray = RAY;
            let ssr = RAY;
            let rho = 0;
    
            // Price: 1 sUSDS = 1.04860 USDS → chi = 1.04860 * RAY
            let chi = 1_048_600_000_000_000_000_000_000_000u128;
            let curve = create_test_curve(ssr, rho, chi, 0);
    
            let unit = 1_000_000u128; // 1 sUSDS
            let source_amount = unit * multiplier as u128;
    
            // Expected USDS: 1.04860 * source_amount
            let expected_destination = source_amount
                .checked_mul(1_048_600u128)
                .unwrap()
                .checked_div(1_000_000u128)
                .unwrap();
    
            // Setup pool with enough liquidity
            let swap_token_b_amount = 1_000_000_000_000u128; // sUSDS
            let swap_token_a_amount = U256::from(swap_token_b_amount)
                .checked_mul(U256::from(chi))
                .unwrap()
                .checked_div(U256::from(ray))
                .unwrap()
                .as_u128(); // USDS
    
            let result = curve
                .swap_without_fees(
                    source_amount,
                    swap_token_b_amount,
                    swap_token_a_amount,
                    TradeDirection::BtoA,
                    Some(0),
                )
                .unwrap();
    
            prop_assert_eq!(result.source_amount_swapped, source_amount);
            prop_assert_eq!(result.destination_amount_swapped, expected_destination);
        }
    }

    proptest! {
        #[test]
        fn test_usds_susds_precision_slippage_scaled(
            multiplier in 1u64..100_000_000u64,
        ) {
            let ray = RAY;
            let ssr = RAY;
            let rho = 0;
            let chi = 1_048_600_000_000_000_000_000_000_000u128; // 1.04860 * RAY
    
            let curve = create_test_curve(ssr, rho, chi, 0);
    
            let unit = 1_000_000u128;
            let source_amount = unit * multiplier as u128;
    
            let expected_destination = U256::from(source_amount)
                .checked_mul(U256::from(ray))
                .unwrap()
                .checked_div(U256::from(chi))
                .unwrap()
                .as_u128();
    
            // Balanced pool
            let swap_token_a_amount = 1_000_000_000_000u128; // USDS
            let swap_token_b_amount = U256::from(swap_token_a_amount)
                .checked_mul(U256::from(ray))
                .unwrap()
                .checked_div(U256::from(chi))
                .unwrap()
                .as_u128(); // sUSDS
    
            let result = curve
                .swap_without_fees(
                    source_amount,
                    swap_token_a_amount,
                    swap_token_b_amount,
                    TradeDirection::AtoB,
                    Some(0),
                )
                .unwrap();
    
            println!("result: {:?}", result);
    
            let actual = result.destination_amount_swapped;
            let diff = if actual > expected_destination {
                actual - expected_destination
            } else {
                expected_destination - actual
            };
    
            prop_assert!(
                diff <= 1,
                "slippage too high: got {}, expected {}, diff {}",
                actual,
                expected_destination,
                diff
            );
        }
    }

    #[test]
    fn test_set_rates_rho_decreasing_boundary() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            ONE_HUNDRED_PCT_APY_SSR
        );

        let curve = curve.set_rates(
            FIVE_PCT_APY_SSR, 
            INITIAL_TIMESTAMP, 
            RAY, 
            INITIAL_TIMESTAMP+1
        ).unwrap();
        
        // Should fail when rho decreases
        assert!(curve.set_rates(
            FIVE_PCT_APY_SSR, 
            INITIAL_TIMESTAMP - 1,
            RAY, 
            INITIAL_TIMESTAMP + 1
        )
        .is_err());
        
        // Should succeed when rho stays the same
        curve.set_rates(
            FIVE_PCT_APY_SSR, 
            INITIAL_TIMESTAMP,
            RAY, 
            INITIAL_TIMESTAMP+1
        )
        .unwrap();
    }

    #[test]
    fn test_set_rates_rho_in_future_boundary() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            0
        );
        
        // Should fail when rho is in the future
        assert!(curve.set_rates(
            FIVE_PCT_APY_SSR, 
            INITIAL_TIMESTAMP + 1, 1_030_000_000_000_000_000_000_000_000, 
            INITIAL_TIMESTAMP
        )
        .is_err());
        
        // Should succeed when rho is current
        curve.set_rates(
            FIVE_PCT_APY_SSR, 
            INITIAL_TIMESTAMP, 
            1_030_000_000_000_000_000_000_000_000, 
            INITIAL_TIMESTAMP
        ).unwrap();
    }

    #[test]
    fn test_set_rates_ssr_below_ray_boundary() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            0
        );
        
        // Should fail when ssr < RAY
        assert!(curve.set_rates(
            RAY - 1, 
            INITIAL_TIMESTAMP, 
            1_030_000_000_000_000_000_000_000_000, 
            INITIAL_TIMESTAMP
        ).is_err());
        
        // Should succeed when ssr == RAY
        curve.set_rates(
            RAY, 
            INITIAL_TIMESTAMP, 
            1_030_000_000_000_000_000_000_000_000, 
            INITIAL_TIMESTAMP
        ).unwrap();
    }

    #[test]
    fn test_set_rates_ssr_above_max_boundary() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            ONE_HUNDRED_PCT_APY_SSR
        );
        
        // Should fail when ssr > max_ssr
        assert!(curve.set_rates(
            ONE_HUNDRED_PCT_APY_SSR + 1, 
            INITIAL_TIMESTAMP, 
            1_030_000_000_000_000_000_000_000_000, 
            INITIAL_TIMESTAMP
        )
        .is_err());
        
        // Should succeed when ssr == max_ssr
        curve.set_rates(
            ONE_HUNDRED_PCT_APY_SSR, 
            INITIAL_TIMESTAMP, 
            1_030_000_000_000_000_000_000_000_000, 
            INITIAL_TIMESTAMP
        )
        .unwrap();
    }

    #[test]
    fn test_set_rates_very_high_ssr_no_max() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            0
        );
        
        // Should succeed with very high SSR when no max is set
        curve.set_rates(
            2 * RAY, 
            INITIAL_TIMESTAMP, 
            1_030_000_000_000_000_000_000_000_000, 
            INITIAL_TIMESTAMP
        )
        .unwrap();
    }

    #[test]
    fn test_set_rates_chi_decreasing_boundary() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            0
        );
        
        // Initial setup
        let curve = curve.set_rates(
            FIVE_PCT_APY_SSR, 
            INITIAL_TIMESTAMP, 
            RAY, 
            INITIAL_TIMESTAMP
        ).unwrap();
        
        // Should fail when chi decreases
        assert!(curve.set_rates(
            FIVE_PCT_APY_SSR, 
            SECOND_TIMESTAMP, RAY - 1, 
            SECOND_TIMESTAMP
        ).is_err());
        
        // Should succeed when chi stays the same
        curve.set_rates(
            FIVE_PCT_APY_SSR, 
            SECOND_TIMESTAMP, 
            RAY, 
            SECOND_TIMESTAMP
        ).unwrap();
    }

    #[test]
    fn test_set_rates_chi_growth_too_fast_boundary() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            ONE_HUNDRED_PCT_APY_SSR
        );
                
        // Set initial values
        let curve = curve.set_rates(
            FIVE_PCT_APY_SSR, 
            INITIAL_TIMESTAMP, 
            RAY, 
            INITIAL_TIMESTAMP
        ).unwrap();

        // Calculate max chi growth for 1 year at max SSR
        let chi_max = curve._rpow(
            ONE_HUNDRED_PCT_APY_SSR,
             SECONDS_PER_YEAR
        ).unwrap();

        let chi_max_u128 = chi_max.as_u128();
        
        // Should fail when chi grows too fast
        assert!(curve.set_rates(
            FIVE_PCT_APY_SSR, 
            SECOND_TIMESTAMP, 
            chi_max_u128 + 1, 
            SECOND_TIMESTAMP
        ).is_err());
        
        // Should succeed at max allowed chi
        curve.set_rates(
            FIVE_PCT_APY_SSR, 
            SECOND_TIMESTAMP, 
            chi_max_u128, 
            SECOND_TIMESTAMP
        ).unwrap();
    }

    #[test]
    fn test_set_rates_chi_large_growth_no_max_ssr() {
        let curve = create_test_curve(
            0, 
            0, 
            0, 
            0
        );
        
        // Should succeed with large chi growth when no max SSR is set
        curve.set_rates(
            FIVE_PCT_APY_SSR, 
            INITIAL_TIMESTAMP, 
            100_000 * RAY, 
            INITIAL_TIMESTAMP
        ).unwrap();
    }

    #[test]
    fn swap_calculation_no_price() {
        let swap_source_amount: u128 = 0;
        let swap_destination_amount: u128 = 0;
        let source_amount: u128 = 100;

        let curve = create_test_curve(RAY, 0, RAY, 0);

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
                Some(0)
            )
            .unwrap();
        assert_eq!(result, expected_result);

        let result = curve
            .swap_without_fees(
                source_amount,
                swap_source_amount,
                swap_destination_amount,
                TradeDirection::BtoA,
                Some(0)
            )
            .unwrap();
        assert_eq!(result, expected_result);
    }

    #[test]
    fn pack_flat_curve() {
        let ssr = RAY;
        let rho = 0;
        let chi = RAY;

        let curve = create_test_curve(ssr, rho, chi, 0);

        let mut packed = [0u8; RedemptionRateCurve::LEN];
        Pack::pack_into_slice(&curve, &mut packed[..]);
        let unpacked = RedemptionRateCurve::unpack(&packed).unwrap();
        assert_eq!(curve, unpacked);

        let mut packed = vec![];
        packed.extend_from_slice(&RAY.to_le_bytes());
        packed.extend_from_slice(&0u128.to_le_bytes());
        packed.extend_from_slice(&ssr.to_le_bytes());
        packed.extend_from_slice(&rho.to_le_bytes());
        packed.extend_from_slice(&chi.to_le_bytes());
        let unpacked = RedemptionRateCurve::unpack(&packed).unwrap();
        assert_eq!(curve, unpacked);
    }

    #[test]
    fn swap_calculation_large_price() {
        let token_b_price = 1_123_513u128;
        let token_b_amount = 500u128;
        let token_a_amount = token_b_amount * token_b_price;

        let chi = token_b_price * RAY;
        let ssr = RAY; // fixed 1.0 rate
        let rho = 0;

        let curve = create_test_curve(ssr, rho, chi, 0);

        // price too low
        let bad_result = curve.swap_without_fees(
            token_b_price - 1,
            token_a_amount,
            token_b_amount,
            TradeDirection::AtoB,
            Some(0),
        );
        assert!(bad_result.is_none());

        // source too small
        let bad_result = curve.swap_without_fees(
            1u128,
            token_a_amount,
            token_b_amount,
            TradeDirection::AtoB,
            Some(0),
        );
        assert!(bad_result.is_none());

        //exact match
        let result = curve
            .swap_without_fees(
                token_b_price,
                token_a_amount,
                token_b_amount,
                TradeDirection::AtoB,
                Some(0),
            )
            .unwrap();

        assert_eq!(result.source_amount_swapped, token_b_price);
        assert_eq!(result.destination_amount_swapped, 1u128);
    }

    #[test]
    fn swap_calculation_max_min() {
        // u64::MAX * RAY wont fit in u128
        let token_b_price = u32::MAX as u128 * RAY;
        let token_b_amount = 1u128;
        let token_a_amount = token_b_price;

        let curve = create_test_curve(1, 0, token_b_price, 0);

        // fails because the source_amount is not enough
        let bad_result = curve.swap_without_fees(
            1,
            token_a_amount,
            token_b_amount,
            TradeDirection::AtoB,
            Some(0)
        );
        assert!(bad_result.is_none());

        let bad_result = curve.swap_without_fees(
            0u128, 
            token_a_amount, 
            token_b_amount, 
            TradeDirection::AtoB, Some(0)
        );
        assert!(bad_result.is_none());

        let result = curve
            .swap_without_fees(
                token_b_price,
                token_a_amount,
                token_b_amount,
                TradeDirection::AtoB,
                Some(0)
            )
            .unwrap();
        println!("result: {:?}", result);
        assert_eq!(result.source_amount_swapped, token_b_price);
        assert_eq!(result.destination_amount_swapped / RAY, 1u128);
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
            chi_raw in 1_000_000..10_000_000_000u128
        ) {

            let ssr = RAY; // fixed interest rate of 1.0
            let rho = 0;
            let chi = chi_raw * RAY;

            let curve = create_test_curve(ssr, rho, chi, 0);
            
            let token_b_price = chi / RAY;
            let source_token_amount = source_token_amount as u128;
            let swap_source_amount = swap_source_amount as u128;
            let swap_destination_amount = swap_destination_amount as u128;

            let traded_source_amount = source_token_amount / 2;
            // Make sure that the trade yields at least 1 token B
            prop_assume!(traded_source_amount / token_b_price >= 1);
            // Make sure there's enough tokens to get back on the other side
            prop_assume!(traded_source_amount / token_b_price <= swap_destination_amount);
            
            check_deposit_token_conversion(
                &curve,
                source_token_amount,
                swap_source_amount,
                swap_destination_amount,
                TradeDirection::AtoB,
                pool_supply,
                CONVERSION_BASIS_POINTS_GUARANTEE,
                Some(0)
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
            chi_raw in 1_000_000..10_000_000_000u128, // keeps price reasonable
        ) {

            let ssr = RAY; // fixed interest rate of 1.0
            let rho = 0;
            let chi = chi_raw * RAY;

            let curve = create_test_curve(ssr, rho, chi, 0);
            
            let token_b_price = chi / RAY;
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
                Some(0)
            );
        }
    }

    proptest! {
        #[test]
        fn withdraw_token_conversion(
            (pool_token_supply, pool_token_amount) in total_and_intermediate(u64::MAX),
            swap_token_a_amount in 1..u64::MAX,
            swap_token_b_amount in 1..u32::MAX, // kept small to avoid proptest rejections
            chi_raw in 1_000_000..10_000_000_000u128, // kept small to avoid proptest rejections
        ) {

            let ssr = RAY; // fixed interest rate of 1.0
            let rho = 0;
            let chi = chi_raw * RAY;
            let token_b_price = chi / RAY;

            let curve = create_test_curve(ssr, rho, chi, 0);

            let pool_token_amount = pool_token_amount as u128;
            let pool_token_supply = pool_token_supply as u128;
            let swap_token_a_amount = swap_token_a_amount as u128;
            let swap_token_b_amount = swap_token_b_amount as u128;

            let value = curve.normalized_value(swap_token_a_amount, swap_token_b_amount, Some(0)).unwrap();

            // Make sure we trade at least one of each token
            prop_assume!(
                U256::from(pool_token_amount) * U256::from(value.to_imprecise().unwrap()) 
                >= 
                U256::from(2) * U256::from(token_b_price) * U256::from(pool_token_supply)
            );

            let withdraw_result = curve
                .pool_tokens_to_trading_tokens(
                    pool_token_amount,
                    pool_token_supply,
                    swap_token_a_amount,
                    swap_token_b_amount,
                    RoundDirection::Floor,
                    Some(0)
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
                CONVERSION_BASIS_POINTS_GUARANTEE * 100,
                Some(0)
            );
            check_withdraw_token_conversion(
                &curve,
                pool_token_amount,
                pool_token_supply,
                swap_token_a_amount,
                swap_token_b_amount,
                TradeDirection::BtoA,
                // TODO see why this needs to be so high
                CONVERSION_BASIS_POINTS_GUARANTEE * 100,
                Some(0)
            );
        }
    }

    proptest! {
        #[test]
        fn curve_value_does_not_decrease_from_swap_a_to_b(
            source_token_amount in 1..u64::MAX,
            swap_source_amount in 1..u64::MAX,
            swap_destination_amount in 1..u64::MAX,
            chi_raw in 1_000_000..10_000_000_000u128,
        ) {
            let ssr = RAY; // fixed interest rate of 1.0
            let rho = 0;
            let chi = chi_raw * RAY;
            let token_b_price = chi / RAY;
            
            let source_token_amount = U256::from(source_token_amount);
            let swap_destination_amount = U256::from(swap_destination_amount);

            // Make sure that the trade yields at least 1 token B
            prop_assume!(source_token_amount / token_b_price >= U256::from(1));
            // Make sure there's enough tokens to get back on the other side
            prop_assume!(source_token_amount / token_b_price <= swap_destination_amount);
            let curve = create_test_curve(ssr, rho, chi, 0);
            check_curve_value_from_swap(
                &curve,
                source_token_amount.as_u128(),
                swap_source_amount as u128,
                swap_destination_amount.as_u128(),
                TradeDirection::AtoB,
                Some(0)
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
            chi_raw in 1_000_000..10_000_000_000u128,
        ) {
            let ssr = RAY; // fixed interest rate of 1.0
            let rho = 0;
            let chi = chi_raw * RAY;
            let token_b_price = chi / RAY;

            let curve = create_test_curve(ssr, rho, chi, 0);

            let pool_token_amount = pool_token_amount as u128;
            let swap_token_a_amount = swap_token_a_amount as u128;
            let swap_token_b_amount = swap_token_b_amount as u128;
            let token_b_price = token_b_price as u128;

            let value = curve.normalized_value(swap_token_a_amount, swap_token_b_amount, Some(0)).unwrap();

            // Make sure we trade at least one of each token
            prop_assume!(
                U256::from(pool_token_amount) * U256::from(value.to_imprecise().unwrap()) 
                >= 
                U256::from(2) * U256::from(token_b_price) * U256::from(pool_token_supply)
            );
            let deposit_result = curve
                .pool_tokens_to_trading_tokens(
                    pool_token_amount,
                    pool_token_supply,
                    swap_token_a_amount,
                    swap_token_b_amount,
                    RoundDirection::Ceiling,
                    Some(0)
                )
                .unwrap();
            let new_swap_token_a_amount = swap_token_a_amount + deposit_result.token_a_amount;
            let new_swap_token_b_amount = swap_token_b_amount + deposit_result.token_b_amount;
            let new_pool_token_supply = pool_token_supply + pool_token_amount;

            let new_value = curve.normalized_value(new_swap_token_a_amount, new_swap_token_b_amount, Some(0)).unwrap();

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
            chi_raw in 1_000_000..10_000_000_000u128, // kept small to avoid proptest rejections
        ) {

            let ssr = RAY; // fixed interest rate of 1.0
            let rho = 0;
            let chi = chi_raw * RAY;
            let token_b_price = chi / RAY;

            let curve = create_test_curve(ssr, rho, chi, 0);

            let pool_token_amount = pool_token_amount as u128;
            let pool_token_supply = pool_token_supply as u128;
            let swap_token_a_amount = swap_token_a_amount as u128;
            let swap_token_b_amount = swap_token_b_amount as u128;
            let token_b_price = token_b_price as u128;

            let value = curve.normalized_value(
                swap_token_a_amount, 
                swap_token_b_amount, 
                Some(0)
            ).unwrap();

            // Make sure we trade at least one of each token
            prop_assume!(
                U256::from(pool_token_amount) * U256::from(value.to_imprecise().unwrap()) 
                >= 
                U256::from(2) * U256::from(token_b_price) * U256::from(pool_token_supply)
            );
            prop_assume!(pool_token_amount <= pool_token_supply);
            let withdraw_result = curve
                .pool_tokens_to_trading_tokens(
                    pool_token_amount,
                    pool_token_supply,
                    swap_token_a_amount,
                    swap_token_b_amount,
                    RoundDirection::Floor,
                    Some(0)
                )
                .unwrap();
            prop_assume!(withdraw_result.token_a_amount <= swap_token_a_amount);
            prop_assume!(withdraw_result.token_b_amount <= swap_token_b_amount);
            let new_swap_token_a_amount = swap_token_a_amount - withdraw_result.token_a_amount;
            let new_swap_token_b_amount = swap_token_b_amount - withdraw_result.token_b_amount;
            let new_pool_token_supply = pool_token_supply - pool_token_amount;

            let new_value = curve.normalized_value(
                new_swap_token_a_amount, 
                new_swap_token_b_amount, 
                Some(0)
            ).unwrap();

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