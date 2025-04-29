//! Curve inspired by Spark PSM3
use arrayref::array_ref;
use solana_program::{
    pubkey::Pubkey,
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack, Sealed},
};
use spl_math::uint::U256;
use crate::error::SwapError;

use super::calculator::{
    map_zero_to_none, CurveCalculator, DynPack, SwapWithoutFeesResult, TradeDirection
};


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
                let destination_amount_swapped = source_amount
                    .checked_mul(ray)?
                    .checked_div(token_b_price)?;

                let mut source_amount_swapped = source_amount;

                let remainder = source_amount_swapped
                    .checked_mul(ray)?
                    .checked_rem(token_b_price)?;

                if remainder > U256::zero() {
                    let reduction = remainder
                        .checked_mul(ray)?
                        .checked_div(token_b_price)?;

                    source_amount_swapped = source_amount.checked_sub(reduction)?;
                }

                (source_amount_swapped, destination_amount_swapped)
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
        _pool_tokens: u128,
        _pool_token_supply: u128,
        _swap_token_a_amount: u128,
        _swap_token_b_amount: u128,
        _round_direction: super::calculator::RoundDirection,
    ) -> Option<super::calculator::TradingTokenResult> {
        todo!()
    }

    fn deposit_single_token_type(
        &self,
        _source_amount: u128,
        _swap_token_a_amount: u128,
        _swap_token_b_amount: u128,
        _pool_supply: u128,
        _trade_direction: TradeDirection,
    ) -> Option<u128> {
        todo!()
    }

    fn withdraw_single_token_type_exact_out(
        &self,
        _source_amount: u128,
        _swap_token_a_amount: u128,
        _swap_token_b_amount: u128,
        _pool_supply: u128,
        _trade_direction: TradeDirection,
        _round_direction: super::calculator::RoundDirection,
    ) -> Option<u128> {
        todo!()
    }

    fn validate(&self) -> Result<(), SwapError> {
        todo!()
    }

    fn validate_supply(
        &self, 
        _token_a_amount: u64, 
        _token_b_amount: u64
    ) -> Result<(), SwapError> {
        todo!()
    }

    fn normalized_value(
        &self,
        _swap_token_a_amount: u128,
        _swap_token_b_amount: u128,
    ) -> Option<spl_math::precise_number::PreciseNumber> {
        todo!()
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
    use std::u128;

    use super::*;

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


}