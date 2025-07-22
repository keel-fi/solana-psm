// SPDX-License-Identifier: AGPL-3.0-only

use solana_program_test::{BanksClient, BanksClientError};
use solana_sdk::{
    hash::Hash, 
    program_pack::Pack, 
    pubkey::Pubkey, 
    signature::Keypair, 
    signer::Signer, 
    system_instruction::{self, create_account}, 
    transaction::Transaction,
};
use solana_program_test::{
    ProgramTest, 
    ProgramTestContext
};
use solana_program::pubkey;
use nova_psm::{
    curve::redemption_rate::RedemptionRateCurve, permission::Permission, state::SwapVersion
};
use spl_token::{
    state::{Mint, Account as TokenAccount},
    ID as TOKEN_PROGRAM_ID,
    instruction::mint_to
};

pub const PROGRAM_ID: Pubkey = pubkey!("5B9vCSSga3qXgHca5Liy3WAQqC2HaB3sBsyjfkH47uYv");

pub async fn program_test_context() -> ProgramTestContext {
    let mut program_test = ProgramTest::default();

    program_test.add_program(
        "nova_psm", 
        PROGRAM_ID, 
        None
    );

    program_test.start_with_context().await
}

pub fn get_permission_pda(
    swap_info: &Pubkey,
    permission_authority: &Pubkey
) -> Pubkey {
    let (address, _bump) = Pubkey::find_program_address(
        &[
            Permission::PERMISSION_SEED,
            &swap_info.to_bytes(),
            &permission_authority.to_bytes()
        ], 
        &PROGRAM_ID
    );
    address
}

pub async fn get_init_curve_setup(
    mut banks_client: &mut BanksClient,
    context_payer: &Keypair,
    last_blockhash: Hash,
    fee_and_destination_owner: &Pubkey
) -> (Pubkey, Pubkey, Pubkey, Pubkey, Pubkey, Pubkey, Pubkey, Pubkey, Pubkey) {
    let swap_info = create_swap_account(
        &mut banks_client, 
        context_payer, 
        last_blockhash
    ).await;

    let (authority, _) = Pubkey::find_program_address(
        &[&swap_info.to_bytes()], 
        &PROGRAM_ID
    );

    let token_a_mint = create_mint(
        &mut banks_client, 
        context_payer, 
        last_blockhash,
        &TOKEN_PROGRAM_ID,
        Some(&context_payer.pubkey()),
        None
    ).await;

    let token_b_mint = create_mint(
        &mut &mut banks_client, 
        &context_payer, 
        last_blockhash, 
        &TOKEN_PROGRAM_ID,
        Some(&context_payer.pubkey()),
        None,
    ).await;

    let pool_mint = create_mint(
        &mut banks_client, 
        context_payer, 
        last_blockhash, 
        &TOKEN_PROGRAM_ID,
        Some(&authority),
        None
    ).await;

    let token_a_account = create_token_account(
        &mut banks_client, 
        last_blockhash, 
        context_payer, 
        &token_a_mint, 
        &authority
    ).await;

    mint_to_token_account(
        &mut banks_client, 
        &TOKEN_PROGRAM_ID, 
        &token_a_mint, 
        context_payer, 
        &token_a_account, 
        1_000_000_000, 
        last_blockhash
    ).await;

    let token_b_account = create_token_account(
        &mut banks_client, 
        last_blockhash, 
        context_payer, 
        &token_b_mint, 
        &authority
    ).await;

    mint_to_token_account(
        &mut banks_client, 
        &TOKEN_PROGRAM_ID, 
        &token_b_mint, 
        context_payer, 
        &token_b_account, 
        1_000_000_000, 
        last_blockhash
    ).await;

    let fee_account = create_token_account(
        &mut banks_client, 
        last_blockhash, 
        context_payer, 
        &pool_mint, 
        &fee_and_destination_owner
    ).await;

    let (destination_owner_pda, _) = Pubkey::find_program_address(
        &[
            b"init_destination", 
            &swap_info.to_bytes()
        ],
        &PROGRAM_ID
    );

    let destination_account = create_token_account(
        &mut banks_client, 
        last_blockhash, 
        context_payer, 
        &pool_mint, 
        &destination_owner_pda
    ).await;
    (swap_info, authority, token_a_mint, token_b_mint, pool_mint, token_a_account, token_b_account, fee_account, destination_account)
}


pub async fn fetch_permission(
    banks_client: &mut BanksClient,
    permission: &Pubkey
) -> Permission {
    let account = banks_client.get_account(*permission)
        .await
        .unwrap()
        .unwrap();

    let permission = Permission::unpack(&account.data)
        .unwrap();

    permission
}

pub async fn fetch_redemption_rate_curve(
    banks_client: &mut BanksClient,
    swap_info: &Pubkey
) -> RedemptionRateCurve {
    let account = banks_client.get_account(*swap_info)
        .await
        .unwrap()
        .unwrap();

    let swap_version = SwapVersion::unpack(&account.data).unwrap();

    let mut calculator_dst = vec![0; 64];
    swap_version.swap_curve().calculator.pack_into_slice(&mut calculator_dst);

    RedemptionRateCurve::unpack_unchecked(
        &calculator_dst
    ).unwrap()
}

async fn create_token_account(
    banks_client: &mut BanksClient,
    last_blockhash: Hash,
    payer: &Keypair,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Pubkey {
    let token_account = Keypair::new();
    let rent = banks_client.get_rent().await.unwrap();
    let lamports = rent.minimum_balance(TokenAccount::LEN);

    let create_account_ix = system_instruction::create_account(
        &payer.pubkey(),
        &token_account.pubkey(),
        lamports,
        TokenAccount::LEN as u64,
        &TOKEN_PROGRAM_ID,
    );

    let init_account_ix = spl_token::instruction::initialize_account(
        &TOKEN_PROGRAM_ID,
        &token_account.pubkey(),
        &mint,
        &owner,
    ).unwrap();
    
    let tx = Transaction::new_signed_with_payer(
        &[create_account_ix, init_account_ix],
        Some(&payer.pubkey()),
        &[payer, &token_account],
        last_blockhash,
    );
    
    banks_client.process_transaction(tx).await.unwrap();

    token_account.pubkey()
}


async fn create_swap_account(
    banks_client: &mut BanksClient,
    payer: &Keypair,
    last_blockhash: Hash,
) -> Pubkey {
    let keypair = Keypair::new();

    let rent = banks_client.get_rent().await.unwrap();
    let len = SwapVersion::LATEST_LEN;

    let init_account_ix = create_account(
        &payer.pubkey(), 
        &keypair.pubkey(), 
        rent.minimum_balance(len), 
        len as u64, 
        &PROGRAM_ID
    );

    let tx = Transaction::new_signed_with_payer(
        &[init_account_ix],
        Some(&payer.pubkey()),
        &[&payer, &keypair],
        last_blockhash,
    );

    banks_client.process_transaction(tx).await.unwrap();

    keypair.pubkey()
}

async fn create_mint(
    banks_client: &mut BanksClient,
    payer: &Keypair,
    last_blockhash: Hash,
    token_program_id: &Pubkey,
    mint_authority: Option<&Pubkey>,
    freeze_authority: Option<&Pubkey>
) -> Pubkey {
    let keypair = Keypair::new();
    let rent = banks_client.get_rent().await.unwrap();

    let init_account_ix = create_account(
        &payer.pubkey(), 
        &keypair.pubkey(), 
        rent.minimum_balance(Mint::LEN), 
        Mint::LEN as u64, 
        token_program_id
    );

    let init_mint_ix = spl_token_2022::instruction::initialize_mint(
        token_program_id, 
        &keypair.pubkey(), 
        mint_authority.unwrap_or(&payer.pubkey()), 
        freeze_authority, 
        9
    ).unwrap();

    let tx = Transaction::new_signed_with_payer(
        &[init_account_ix, init_mint_ix],
        Some(&payer.pubkey()),
        &[&payer, &keypair],
        last_blockhash,
    );

    banks_client.process_transaction(tx).await.unwrap();

    keypair.pubkey()

}


async fn mint_to_token_account(
    banks_client: &mut BanksClient,
    token_program_id: &Pubkey,
    mint: &Pubkey, 
    authority: &Keypair, 
    user_ata: &Pubkey, 
    amount: u64,
    last_blockhash: Hash,
) {
    let ix = mint_to(
        token_program_id, 
        mint, 
        user_ata, 
        &authority.pubkey(), 
        &[], 
        amount
    ).unwrap();

    let tx = Transaction::new_signed_with_payer(
        &[ix], 
        Some(&authority.pubkey()), 
        &[authority], 
        last_blockhash
    );

    banks_client.process_transaction(tx).await.unwrap();
}

pub async fn get_transaction_simulation_cu_used(
    context: &mut ProgramTestContext,
    transaction: Transaction
) -> Result<u64, BanksClientError> {
    let simulation_result = context.banks_client
        .simulate_transaction(transaction)
        .await?;

    let details = simulation_result.simulation_details.unwrap();

    let cu_used = details.units_consumed;

    Ok(cu_used)
}