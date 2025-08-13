// SPDX-License-Identifier: AGPL-3.0-only

use {
    crate::native_account_data::NativeAccountData,
    solana_program::{
        account_info::AccountInfo, 
        clock::{Clock, Epoch, Slot, UnixTimestamp}, 
        entrypoint::ProgramResult, instruction::Instruction, 
        program_error::ProgramError, 
        program_stubs, pubkey::Pubkey, rent::Rent,
        system_program
    },
    std::sync::{Arc, Mutex}
};

struct TestSyscallStubs {
    clock: Arc<Mutex<Clock>>,
    rent: Arc<Mutex<Rent>>
}

impl TestSyscallStubs {
    pub fn new(clock: Clock, rent: Rent) -> Self {
        TestSyscallStubs {
            clock: Arc::new(Mutex::new(clock)),
            rent: Arc::new(Mutex::new(rent))
        }
    }
}

impl program_stubs::SyscallStubs for TestSyscallStubs {
    fn sol_invoke_signed(
        &self,
        instruction: &Instruction,
        account_infos: &[AccountInfo],
        signers_seeds: &[&[&[u8]]],
    ) -> ProgramResult {
        let mut new_account_infos = vec![];

        // mimic check for token program in accounts
        if instruction.program_id == spl_token::id()
            && !account_infos.iter().any(|x| *x.key == spl_token::id())
        {
            return Err(ProgramError::InvalidAccountData);
        }

        for meta in instruction.accounts.iter() {
            for account_info in account_infos.iter() {
                if meta.pubkey == *account_info.key {
                    let mut new_account_info = account_info.clone();
                    for seeds in signers_seeds.iter() {
                        let signer =
                            Pubkey::create_program_address(seeds, &solana_psm::id()).unwrap();
                        if *account_info.key == signer {
                            new_account_info.is_signer = true;
                        }
                    }
                    new_account_infos.push(new_account_info);
                }
            }
        }

        if instruction.program_id == spl_token::id() {
            spl_token::processor::Processor::process(
                &instruction.program_id,
                &new_account_infos,
                &instruction.data,
            )
        } else if instruction.program_id == system_program::id() {
            // Handle System Program instructions
            Ok(())
        } else {
            Err(ProgramError::IncorrectProgramId)
        }
    }

    fn sol_get_clock_sysvar(&self, var_addr: *mut u8) -> u64 {
        let clock = self.clock.clone();

        let serialized = bincode::serialize(&*clock).expect("Failed to serialize Clock");

        unsafe {
            std::ptr::copy_nonoverlapping(
                serialized.as_ptr(),
                var_addr,
                serialized.len(),
            );
        }

        0
    }

    fn sol_get_rent_sysvar(&self, var_addr: *mut u8) -> u64 {
        let rent = self.rent.clone();

        let serialized = bincode::serialize(&*rent).expect("Failed to serialize Rent");

        unsafe {
            std::ptr::copy_nonoverlapping(
                serialized.as_ptr(),
                var_addr,
                serialized.len(),
            );
        }

        0
    }
}

fn test_syscall_stubs() {
    use std::sync::Once;
    static ONCE: Once = Once::new();

    ONCE.call_once(|| {

        let mock_clock = Clock {
            slot: 1000 as Slot,
            epoch_start_timestamp: 1625097600 as UnixTimestamp,
            epoch: 10 as Epoch,
            leader_schedule_epoch: 11 as Epoch,
            unix_timestamp: 1625097600 as UnixTimestamp,
        };

        let mock_rent = Rent {
            lamports_per_byte_year: 100,
            exemption_threshold: 1.0,
            burn_percent: 1
        };

        program_stubs::set_syscall_stubs(Box::new(TestSyscallStubs::new(mock_clock, mock_rent)));
    });
}

pub fn do_process_instruction(instruction: Instruction, accounts: &[AccountInfo]) -> ProgramResult {
    test_syscall_stubs();

    // approximate the logic in the actual runtime which runs the instruction
    // and only updates accounts if the instruction is successful
    let mut account_data = accounts
        .iter()
        .map(NativeAccountData::new_from_account_info)
        .collect::<Vec<_>>();
    let account_infos = account_data
        .iter_mut()
        .map(NativeAccountData::as_account_info)
        .collect::<Vec<_>>();
    let res = if instruction.program_id == solana_psm::id() {
        solana_psm::processor::Processor::process(
            &instruction.program_id,
            &account_infos,
            &instruction.data,
        )
    } else {
        spl_token::processor::Processor::process(
            &instruction.program_id,
            &account_infos,
            &instruction.data,
        )
    };

    if res.is_ok() {
        let mut account_metas = instruction
            .accounts
            .iter()
            .zip(accounts)
            .map(|(account_meta, account)| (&account_meta.pubkey, account))
            .collect::<Vec<_>>();
        for account_info in account_infos.iter() {
            for account_meta in account_metas.iter_mut() {
                if account_info.key == account_meta.0 {
                    let account = &mut account_meta.1;
                    let mut lamports = account.lamports.borrow_mut();
                    **lamports = **account_info.lamports.borrow();
                    let mut data = account.data.borrow_mut();
                    data.clone_from_slice(*account_info.data.borrow());
                }
            }
        }
    }
    res
}
