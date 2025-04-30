//! Curve inspired by Spark PSM3
use arrayref::array_ref;
use solana_program::{
    pubkey::Pubkey,
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack, Sealed},
};
use spl_math::{
    checked_ceil_div::CheckedCeilDiv, precise_number::PreciseNumber, uint::U256
};
use crate::error::SwapError;

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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedemptionRateCurve {
    /// Authority allowed to update the SSR parameters.
    pub authority: Pubkey,
    /// Fixed-point scaling factor, typically 1e27 (RAY).
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

    fn _rpow(
        &self,
        x: u128,
        n: u128,
    ) -> Option<U256> {
        let mut z: U256;
        let x_u256 = U256::from(x);
        let n_u256 = U256::from(n);
        let ray_u256 = U256::from(self.ray);
        let half = ray_u256 / U256::from(2);

        if x_u256 == U256::zero() {
            if n_u256 == U256::zero() {
                z = ray_u256;
            } else {
                z = U256::zero();
            }
        } else {
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
    const LEN: usize = 112;

    fn pack_into_slice(&self, output: &mut [u8]) {
        (self as &dyn DynPack).pack_into_slice(output);
    }

    fn unpack_from_slice(input: &[u8]) -> Result<RedemptionRateCurve, ProgramError> {
        let authority = array_ref![input, 0, 32];
        let ray = array_ref![input, 32, 16];
        let max_ssr = array_ref![input, 48, 16];
        let ssr = array_ref![input, 64, 16];
        let rho = array_ref![input, 80, 16];
        let chi = array_ref![input, 96, 16];

        Ok(Self {
            authority: Pubkey::new_from_array(*authority),
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
        let (authority, rest) = output.split_at_mut(32);
        let (ray, rest) = rest.split_at_mut(16);
        let (max_ssr, rest) = rest.split_at_mut(16);
        let (ssr, rest) = rest.split_at_mut(16);
        let (rho, rest) = rest.split_at_mut(16);
        let (chi, _) = rest.split_at_mut(16);

        authority.copy_from_slice(&self.authority.to_bytes());
        ray.copy_from_slice(&self.ray.to_le_bytes());
        max_ssr.copy_from_slice(&self.max_ssr.to_le_bytes());
        ssr.copy_from_slice(&self.ssr.to_le_bytes());
        rho.copy_from_slice(&self.rho.to_le_bytes());
        chi.copy_from_slice(&self.chi.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::curve::calculator::{
            test::{
                check_curve_value_from_swap, check_deposit_token_conversion, check_withdraw_token_conversion, total_and_intermediate, CONVERSION_BASIS_POINTS_GUARANTEE
            },
            INITIAL_SWAP_POOL_AMOUNT,
        },
        proptest::prelude::*,
    };

    const RAY: u128 = 10u128.pow(27);

    fn create_test_curve(
        ssr: u128,
        rho: u128,
        chi: u128,
    ) -> RedemptionRateCurve {
        RedemptionRateCurve {
            authority: Pubkey::default(),
            ray: RAY, 
            max_ssr: 0,
            ssr,
            rho,
            chi,
        }
    }

    #[test]
    fn swap_calculation_no_price() {
        let swap_source_amount: u128 = 0;
        let swap_destination_amount: u128 = 0;
        let source_amount: u128 = 100;

        let curve = create_test_curve(RAY, 0, RAY);

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

        let curve = create_test_curve(ssr, rho, chi);

        let mut packed = [0u8; RedemptionRateCurve::LEN];
        Pack::pack_into_slice(&curve, &mut packed[..]);
        let unpacked = RedemptionRateCurve::unpack(&packed).unwrap();
        assert_eq!(curve, unpacked);

        let mut packed = vec![];
        packed.extend_from_slice(&Pubkey::default().to_bytes());
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

        let curve = create_test_curve(ssr, rho, chi);

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

        let curve = create_test_curve(1, 0, token_b_price);

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

            let curve = create_test_curve(ssr, rho, chi);
            
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

            let curve = create_test_curve(ssr, rho, chi);
            
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

            let curve = create_test_curve(ssr, rho, chi);

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
            let curve = create_test_curve(ssr, rho, chi);
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

            let curve = create_test_curve(ssr, rho, chi);

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

            let curve = create_test_curve(ssr, rho, chi);

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