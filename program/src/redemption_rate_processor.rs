// SPDX-License-Identifier: AGPL-3.0-only

//! Processor for all RedemptionRateCurve instructions.

use std::sync::Arc;

use arrayref::{array_ref, array_refs};
use solana_program::{
    pubkey::Pubkey,
    program_error::ProgramError,
    account_info::{AccountInfo, next_account_info},
    program_pack::Pack,
    clock::Clock,
    sysvar::Sysvar
};

use crate::{
    curve::{
        base::{CurveType, SwapCurve}, 
        calculator::CurveCalculator, 
        redemption_rate::RedemptionRateCurve
    }, permission::Permission, state::{SwapState, SwapV1, SwapVersion}
};

/// Processes update
pub fn process_curve_update(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    ssr: u128,
    rho: u128,
    chi: u128
) -> Result<(), ProgramError> {
    let accounts_info_iter = &mut accounts.iter();

    let swap_info = next_account_info(accounts_info_iter)?;
    let permission_info = next_account_info(accounts_info_iter)?;
    let signer_info = next_account_info(accounts_info_iter)?;

    if swap_info.owner != program_id {
        return Err(ProgramError::IllegalOwner)
    }

    if !signer_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature)
    }

    let permission = Permission::unpack_permission(
        permission_info, 
        swap_info, 
        signer_info, 
        program_id
    )?;
    
    permission.validate_update_params_permission()?;

    let mut swap_data = swap_info.data.borrow_mut();
    let swap = SwapVersion::unpack(&swap_data)?;
    let curve = extract_curve(&swap_data)?;

    let new_swap_state = create_new_swap_state(
        ssr, 
        rho, 
        chi, 
        curve, 
        swap
    )?;

    SwapVersion::pack(new_swap_state, &mut swap_data)?;

    Ok(())
}

fn create_new_swap_state(
    ssr: u128,
    rho: u128,
    chi: u128,
    curve: RedemptionRateCurve,
    swap: Arc<dyn SwapState>,
) -> Result<SwapVersion, ProgramError> {

    let current_timestamp = Clock::get()?.unix_timestamp as u128;

    let new_calculator = curve.set_rates(
        ssr, 
        rho,
        chi, 
        current_timestamp
    )?;

    let new_swap = SwapVersion::SwapV1(SwapV1 {
        is_initialized: swap.is_initialized(),
        bump_seed: swap.bump_seed(),
        token_program_id: swap.token_program_id().clone(),
        token_a: swap.token_a_account().clone(),
        token_b: swap.token_b_account().clone(),
        pool_mint: swap.pool_mint().clone(),
        token_a_mint: swap.token_a_mint().clone(),
        token_b_mint: swap.token_b_mint().clone(),
        pool_fee_account: swap.pool_fee_account().clone(),
        fees: swap.fees().clone(),
        swap_curve: SwapCurve {
            curve_type: CurveType::RedemptionRateCurve,
            calculator: Arc::new(new_calculator) as Arc<dyn CurveCalculator + Send + Sync>,
        },
    });

    Ok(new_swap)

}


fn extract_curve(
    input: &[u8]
) -> Result<RedemptionRateCurve, ProgramError> {
    // equal to SwapVersion::LATEST_LEN - SwapCurve::LEN , SwapCurve::LEN
    let input = array_ref![input, SwapVersion::LATEST_LEN - SwapCurve::LEN, SwapCurve::LEN];

    let (curve_type, calculator) = array_refs![input, 1, RedemptionRateCurve::LEN];

    let curve_type = curve_type[0].try_into()?;

    Ok(match curve_type {
        CurveType::RedemptionRateCurve => {
            RedemptionRateCurve::unpack_from_slice(calculator)?
        },
        _ => return Err(ProgramError::InvalidAccountData)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swap_curve_len() {
        assert_eq!(SwapCurve::LEN, 81);
        assert_eq!(SwapCurve::LEN - 1, RedemptionRateCurve::LEN);
    }

    #[test]
    fn test_swap_v1_curve_offset() {
        // requires that SwapCurve is packed last in SwapVersion
        assert_eq!(SwapVersion::LATEST_LEN - SwapCurve::LEN, 291);
    }
}