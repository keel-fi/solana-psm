//! Test for testing redemption rate curve authority features. 

use helpers::{
    fetch_redemption_rate_curve, 
    get_init_curve_setup, 
    program_test_context,
    PROGRAM_ID
};
use solana_sdk::{
    clock::Clock, 
    instruction::{AccountMeta, Instruction}, 
    program_pack::Pack, 
    pubkey::Pubkey, 
    signature::Keypair, 
    signer::Signer, 
    transaction::Transaction
};
use spl_token_swap::curve::{
    redemption_rate::RedemptionRateCurve, 
    fees::Fees
};
use spl_token::ID as TOKEN_PROGRAM_ID;

mod helpers;

const RAY: u128 = 10u128.pow(27);

#[tokio::test]
async fn test_redemption_rate_curve_authority_update() {
    let mut context = program_test_context().await;

    let authority_keypair = Keypair::new();
    let fake_authority_keypair = Keypair::new();

    let fee_and_destination_owner = Pubkey::new_unique();

    let (
        swap_info,
        authority,
        _token_a_mint,
        _token_b_mint,
        pool_mint,
        token_a_account,
        token_b_account,
        fee_account,
        destination_account
    ) = get_init_curve_setup(
        &mut context.banks_client,
        &context.payer,
        context.last_blockhash,
        &fee_and_destination_owner
    ).await;

    let init_accounts = vec![
        // swap info
        AccountMeta::new(swap_info, false),
        // authority info
        AccountMeta::new_readonly(authority, false),
        // token a info
        AccountMeta::new_readonly(token_a_account, false),
        // token b info
        AccountMeta::new_readonly(token_b_account, false),
        // pool mint info
        AccountMeta::new(pool_mint, false),
        // fee_account info
        AccountMeta::new_readonly(fee_account, false),
        // destination info
        AccountMeta::new(destination_account, false),
        // pool token program info
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
    ];

    let fees = Fees::default();
    let mut fees_buf = vec![0; 64];
    fees.pack_into_slice(&mut fees_buf);

    let clock: Clock = context
        .banks_client
        .get_sysvar::<Clock>()
        .await
        .unwrap();

    let curve_for_creation = RedemptionRateCurve {
        update_authority: authority_keypair.pubkey(),
        ray: RAY,
        max_ssr: 0,
        ssr: RAY,
        rho: clock.unix_timestamp as u128,
        chi: RAY
    };

    let mut calculator_buf = vec![0; 112];
    Pack::pack_into_slice(
        &curve_for_creation, 
        &mut calculator_buf
    );

    let initialize_swap_curve_data = vec![
        // discriminator for SwapInstruction::Initialize
        vec![0],
        fees_buf,
        // discriminator for CurveType::RedemptionRateCurve
        vec![3],
        calculator_buf
    ].concat();

    let init_curve_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: init_accounts,
        data: initialize_swap_curve_data
    };

    let init_curve_tx = Transaction::new_signed_with_payer(
        &[init_curve_ix], 
        Some(&context.payer.pubkey()), 
        &[&context.payer], 
        context.last_blockhash
    );

    context.banks_client
        .process_transaction(init_curve_tx)
        .await
        .unwrap();

    let created_curve = fetch_redemption_rate_curve(
        &mut context.banks_client, 
        &swap_info
    ).await;

    assert_eq!(created_curve, curve_for_creation);

    // update curve with valid authority

    let update_curve_accounts = vec![
        // swap info
        AccountMeta::new(swap_info, false),
        // update_authority
        AccountMeta::new_readonly(authority_keypair.pubkey(), true),
    ];

    let new_ssr: u128 = 2 * RAY;
    let new_rho: u128 = clock.unix_timestamp as u128;
    let new_chi: u128 = 2 * RAY;

    let update_curve_data = vec![
        // discriminator for SwapInstruction::SetRates
        vec![6],
        new_ssr.to_le_bytes().to_vec(),
        new_rho.to_le_bytes().to_vec(),
        new_chi.to_le_bytes().to_vec(),
    ].concat();

    let update_curve_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: update_curve_accounts.clone(),
        data: update_curve_data
    };

    let update_curve_tx = Transaction::new_signed_with_payer(
        &[update_curve_ix], 
        Some(&context.payer.pubkey()), 
        &[&context.payer , &authority_keypair], 
        context.last_blockhash
    );

    let result = context.banks_client
        .process_transaction(update_curve_tx)
        .await;

    assert!(result.is_ok());

    let updated_curve = fetch_redemption_rate_curve(
        &mut context.banks_client, 
        &swap_info
    ).await;
    
    assert_eq!(updated_curve.ssr, new_ssr);
    assert_eq!(updated_curve.rho, new_rho);
    assert_eq!(updated_curve.chi, new_chi);

    // fails at updating curve price with invalid rho
    let invalid_new_rho = clock.unix_timestamp as u128 - 1;
    
    let invalid_rho_data = vec![
        // discriminator for SwapInstruction::SetRates
        vec![6],
        new_ssr.to_le_bytes().to_vec(),
        invalid_new_rho.to_le_bytes().to_vec(),
        new_chi.to_le_bytes().to_vec(),
    ].concat();

    let invalid_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: update_curve_accounts,
        data: invalid_rho_data
    };

    let invalid_tx = Transaction::new_signed_with_payer(
        &[invalid_ix], 
        Some(&context.payer.pubkey()), 
        &[&context.payer , &authority_keypair], 
        context.last_blockhash
    );

    let result = context.banks_client
        .process_transaction(invalid_tx)
        .await;

    assert!(result.is_err());

    // // fails at updating curve price with unauthorized signer

    let unauthorized_update_accounts = vec![
        // swap info
        AccountMeta::new(swap_info, false),
        // update_authority
        AccountMeta::new_readonly(fake_authority_keypair.pubkey(), true),
    ];

    let new_ssr: u128 = 2 * RAY;
    let new_rho: u128 = clock.unix_timestamp as u128;
    let new_chi: u128 = 2 * RAY;

    let update_data = vec![
        // discriminator for SwapInstruction::SetRates
        vec![6],
        new_ssr.to_le_bytes().to_vec(),
        new_rho.to_le_bytes().to_vec(),
        new_chi.to_le_bytes().to_vec(),

    ].concat();

    let invalid_update_price_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: unauthorized_update_accounts,
        data: update_data
    };

    let invalid_update_price_tx = Transaction::new_signed_with_payer(
        &[invalid_update_price_ix], 
        Some(&context.payer.pubkey()), 
        &[&context.payer, &fake_authority_keypair], 
        context.last_blockhash
    );

    let result = context.banks_client
        .process_transaction(invalid_update_price_tx)
        .await;

    assert!(result.is_err());
}