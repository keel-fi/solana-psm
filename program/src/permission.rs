// SPDX-License-Identifier: AGPL-3.0-only

//! Permission system for managing updates.

use solana_program::{
    pubkey::Pubkey,
    program_pack::{IsInitialized, Pack, Sealed},
    program_error::ProgramError,
    system_instruction::{create_account, transfer, assign, allocate},
    sysvar::{Sysvar, rent::Rent},
    account_info::AccountInfo,
    program::{invoke_signed, invoke},
    system_program::ID as SYSTEM_PROGRAM_ID,
    account_info::next_account_info,
};
use arrayref::array_ref;

use crate::{error::SwapError, ID as PROGRAM_ID};

/// Permission struct that allows a more flexiple permission system
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Permission {
    /// The Swap account address
    pub swap: Pubkey,
    /// The pubkey that is granted these permissions and must sign relevant instructions
    pub authority: Pubkey,
    /// A permission to grant or revoke other permissions
    pub is_super_admin: bool,
    /// A permission allowing authority to update curve parameters
    pub can_update_parameters: bool,
}

impl IsInitialized for Permission {
    fn is_initialized(&self) -> bool {
        true
    }
}

impl Sealed for Permission {}

impl Pack for Permission {
    const LEN: usize = 66;

    fn unpack_from_slice(input: &[u8]) -> Result<Permission, ProgramError> {
        let swap = array_ref![input, 0, 32];
        let authority = array_ref![input, 32, 32];
        let is_super_admin = array_ref![input, 64, 1];
        let can_update_parameters = array_ref![input, 65, 1];

        Ok(Self {
            swap: Pubkey::new_from_array(*swap),
            authority: Pubkey::new_from_array(*authority),
            is_super_admin: is_super_admin[0] != 0,
            can_update_parameters: can_update_parameters[0] != 0,
        })
    }

    fn pack_into_slice(&self, output: &mut [u8]) {
        let (swap, rest) = output.split_at_mut(32);
        let (authority, rest) = rest.split_at_mut(32);
        let (is_super_admin, rest) = rest.split_at_mut(1);
        let (can_update_parameters, _) = rest.split_at_mut(1);

        swap.copy_from_slice(&self.swap.to_bytes());
        authority.copy_from_slice(&self.authority.to_bytes());
        is_super_admin[0] = self.is_super_admin as u8;
        can_update_parameters[0] = self.can_update_parameters as u8;
    }
}

impl Permission {

    /// Seed for PDA
    pub const PERMISSION_SEED: &'static [u8] = b"permission";


    /// Derives Permission address based on swap and authority
    fn derive_permission_pubkey_and_bump(
        swap: &Pubkey,
        authority: &Pubkey
    ) -> (Pubkey, u8) {
        let (pubkey, bump) = Pubkey::find_program_address(
            &[
                Self::PERMISSION_SEED,
                &swap.to_bytes(),
                &authority.to_bytes()
            ], 
            &PROGRAM_ID
        );

        (pubkey, bump)
    }

    /// validates that the signer and its permission can update params
    pub fn validate_update_params_permission(
        &self,
        swap_info: &AccountInfo,
        signer_info: &AccountInfo
    ) -> Result<(), ProgramError> {
        self.validate_signer_and_permission(swap_info, signer_info)?;

        if !self.can_update_parameters {
            return Err(SwapError::InvalidUpdatePermission.into())
        }
        
        Ok(())
    }

    /// validates that the signer and its permission is super admin
    pub fn validate_super_admin_permission(
        &self,
        swap_info: &AccountInfo,
        signer_info: &AccountInfo
    ) -> Result<(), ProgramError> {
        self.validate_signer_and_permission(swap_info, signer_info)?;

        if !self.is_super_admin {
            return Err(SwapError::InvalidUpdatePermission.into())
        }

        Ok(())
    }

    /// checks that signer and its permission are valid
    fn validate_signer_and_permission(
        &self,
        swap_info: &AccountInfo,
        signer_info: &AccountInfo
    ) -> Result<(), ProgramError> {
        if !signer_info.is_signer {
            return Err(ProgramError::MissingRequiredSignature)
        }

        if &self.swap != swap_info.key {
            return Err(SwapError::InvalidUpdatePermission.into())
        }

        if self.authority != *signer_info.key {
            return Err(SwapError::InvalidUpdatePermission.into())
        }

        Ok(())
    }

    /// creates the permission account, based of the swap and authority
    pub fn create_permission_account<'a>(
        payer: AccountInfo<'a>,
        permission_account: AccountInfo<'a>,
        system_program: AccountInfo<'a>,
        swap: &Pubkey,
        permission_authority: &Pubkey
    ) -> Result<(), ProgramError> {

        let (
            permission_address,
            permission_bump
        ) = Self::derive_permission_pubkey_and_bump(swap, permission_authority);

        if *permission_account.key != permission_address {
            return Err(SwapError::InvalidPermissionAddress.into())
        }

        if *system_program.key != SYSTEM_PROGRAM_ID {
            return Err(ProgramError::IncorrectProgramId)
        }

        let rent = Rent::get()?;
        let space = Permission::LEN;
        let lamports = rent.minimum_balance(space);

        let current_lamports = permission_account.lamports();

        let signers_seeds: &[&[&[u8]]] = &[&[
            Self::PERMISSION_SEED,
            swap.as_ref(),
            permission_authority.as_ref(),
            &[permission_bump]
        ]];

        // Permission account has no lamports, normal account creation
        if current_lamports == 0 {
            let ix = create_account(
                payer.key, 
                &permission_address, 
                lamports, 
                space as u64, 
                &PROGRAM_ID
            );

            invoke_signed(
                &ix, 
                &[
                    payer, 
                    permission_account, 
                    system_program
                ], 
                signers_seeds
            )?;

            return Ok(())
        }

        // Permission account has a balance, so we have to:
        // 1. Transfer required lamports for rent exempt (if needed)
        // 2. Allocate space for Permission account
        // 3. Assign to current program

        let required_lamports = lamports.max(1)
            .saturating_sub(current_lamports);

        if required_lamports > 0 {
            let ix = transfer(
                payer.key, 
                &permission_address, 
                required_lamports
            );

            invoke(
                &ix, 
                &[
                    payer.clone(), 
                    permission_account.clone(), 
                    system_program.clone()
                ]
            )?;
        }

        let allocate_ix = allocate(
            &permission_address, 
            space as u64
        );

        invoke_signed(
            &allocate_ix, 
            &[
                permission_account.clone(), 
                system_program.clone()
            ], 
            signers_seeds
        )?;

        let assign_ix = assign(
            &permission_address, 
            &PROGRAM_ID
        );

        invoke_signed(
            &assign_ix, 
            &[
                permission_account, 
                system_program
            ], 
            signers_seeds
        )?;

        Ok(())
    }
}

/// Processes initialization of new permission
pub fn process_initialize_permission(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    permission_authority: Pubkey,
    is_super_admin: bool,
    can_update_parameters: bool
) -> Result<(), ProgramError> {
    let accounts_info_iter = &mut accounts.iter();

    let swap_info = next_account_info(accounts_info_iter)?;
    let permission_info = next_account_info(accounts_info_iter)?;
    let new_permission_info = next_account_info(accounts_info_iter)?;
    let signer_info = next_account_info(accounts_info_iter)?;
    let payer_info = next_account_info(accounts_info_iter)?;
    let system_program_info = next_account_info(accounts_info_iter)?;

    if swap_info.owner != program_id {
        return Err(ProgramError::IllegalOwner)
    }

    if permission_info.owner != program_id {
        return Err(ProgramError::IllegalOwner)
    }

    let permission = Permission::unpack(&permission_info.data.borrow())?;
    permission.validate_super_admin_permission(swap_info, signer_info)?;

    let new_permission = Permission {
        swap: *swap_info.key,
        authority: permission_authority,
        is_super_admin,
        can_update_parameters
    };

    Permission::create_permission_account(
        payer_info.clone(), 
        new_permission_info.clone(), 
        system_program_info.clone(), 
        swap_info.key, 
        &permission_authority
    )?;

    Permission::pack(new_permission, &mut new_permission_info.data.borrow_mut())?;

    Ok(())
}

/// Processes permission updates.
/// `permission_info` being equal to `update_permission_info`
/// is valid for revoking own role.
pub fn process_update_permission(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    is_super_admin: bool,
    can_update_parameters: bool
) -> Result<(), ProgramError> {
    let accounts_info_iter = &mut accounts.iter();

    let swap_info = next_account_info(accounts_info_iter)?;
    let permission_info = next_account_info(accounts_info_iter)?;
    let update_permission_info = next_account_info(accounts_info_iter)?;
    let signer_info = next_account_info(accounts_info_iter)?;

    if swap_info.owner != program_id {
        return Err(ProgramError::IllegalOwner)
    }

    if permission_info.owner != program_id {
        return Err(ProgramError::IllegalOwner)
    }

    if update_permission_info.owner != program_id {
        return Err(ProgramError::IllegalOwner)
    }

    let permission: Permission = Permission::unpack(&permission_info.data.borrow())?;
    permission.validate_super_admin_permission(swap_info, signer_info)?;
    
    let mut update_permission_data = update_permission_info.data.borrow_mut();

    let update_permission = Permission::unpack(&update_permission_data)?;

    if update_permission.swap != *swap_info.key {
        return Err(SwapError::InvalidUpdatePermission.into())
    }

    let updated_values = Permission {
        swap: update_permission.swap,
        authority: update_permission.authority,
        is_super_admin,
        can_update_parameters
    };

    Permission::pack(updated_values, &mut update_permission_data)?;

    Ok(())
}