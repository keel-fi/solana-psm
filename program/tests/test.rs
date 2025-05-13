// SPDX-License-Identifier: AGPL-3.0-only

//! Test for testing redemption rate curve authority features. 

use helpers::{
    fetch_permission, 
    fetch_redemption_rate_curve, 
    get_init_curve_setup, 
    get_permission_pda, 
    get_transaction_simulation_cu_used, 
    program_test_context, 
    PROGRAM_ID
};
use solana_program_test::ProgramTestContext;
use solana_sdk::{
    clock::Clock, 
    compute_budget::ComputeBudgetInstruction, 
    instruction::{AccountMeta, Instruction}, 
    program_pack::Pack, 
    pubkey::Pubkey, 
    signature::Keypair, 
    signer::Signer, 
    system_program::ID as SYSTEM_PROGRAM_ID, 
    transaction::Transaction
};
use nova_psm::curve::{
    redemption_rate::RedemptionRateCurve, 
    fees::Fees
};
use spl_token::ID as TOKEN_PROGRAM_ID;

mod helpers;

const RAY: u128 = 10u128.pow(27);
const FIVE_PCT_APY_SSR: u128 = 1_000_000_001_547_125_957_863_212_448;
const ONE_HUNDRED_PCT_APY_SSR: u128 = 1_000_000_021_979_553_151_239_153_020;

#[tokio::test]
async fn test_redemption_rate_curve_creation_and_update() {
    let mut context = program_test_context().await;
    let authority_keypair = Keypair::new();
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

    create_redemption_rate_curve(
        &mut context,
        &swap_info,
        &authority,
        &authority_keypair,
        token_a_account,
        token_b_account,
        pool_mint,
        fee_account,
        destination_account,
        0
    ).await;

    test_curve_update_valid(
        &mut context, 
        &swap_info, 
        &authority_keypair
    ).await;
    
    test_curve_update_invalid_rho(
        &mut context, 
        &swap_info, 
        &authority_keypair
    ).await;
}

#[tokio::test]
async fn test_redemption_curve_permission_system() {
    let mut context = program_test_context().await;
    let authority_keypair = Keypair::new();
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

    let permission_account = get_permission_pda(
        &swap_info, 
        &authority_keypair.pubkey()
    );

    create_redemption_rate_curve(
        &mut context,
        &swap_info,
        &authority,
        &authority_keypair,
        token_a_account,
        token_b_account,
        pool_mint,
        fee_account,
        destination_account,
        0
    ).await;

    test_permission_metadata(
        &mut context,
        &permission_account, 
        &swap_info, 
        &authority_keypair
    ).await;

    test_permission_rejection_unauthorized_update(
        &mut context, 
        &swap_info, 
        &permission_account
    ).await;

    test_permission_grant_and_update(
        &mut context, 
        &swap_info, 
        &permission_account, 
        &authority_keypair
    ).await;
}

#[tokio::test]
async fn test_rpow_performace_with_duration() {
    let mut context = program_test_context().await;
    let authority_keypair = Keypair::new();
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

    create_redemption_rate_curve(
        &mut context,
        &swap_info,
        &authority,
        &authority_keypair,
        token_a_account,
        token_b_account,
        pool_mint,
        fee_account,
        destination_account,
        ONE_HUNDRED_PCT_APY_SSR
    ).await;

    test_rpow_compute_units_with_growing_duration(
        &mut context,
        &swap_info,
        &authority_keypair
    ).await
}

#[tokio::test]
async fn test_rpow_performance_with_different_max_ssr() {
    // Define precise APY values
    const ONE_PCT_APY_SSR: u128 = 1_000_000_000_314_714_530_356_867_391;
    const FIFTEEN_PCT_APY_SSR: u128 = 1_000_000_004_432_554_513_667_376_032;
    const TWENTY_PCT_APY_SSR: u128 = 1_000_000_005_860_733_888_492_302_697;
    const TWENTY_FIVE_PCT_APY_SSR: u128 = 1_000_000_007_264_617_216_271_247_405;
    
    // Create a test matrix with different max_ssr values
    let max_ssr_values = vec![
        (ONE_PCT_APY_SSR, "1% APY compound"),
        (FIVE_PCT_APY_SSR, "5% APY compound"),
        (FIFTEEN_PCT_APY_SSR, "15% APY compound"),
        (TWENTY_PCT_APY_SSR, "20% APY compound"),
        (TWENTY_FIVE_PCT_APY_SSR, "25% APY compound"),
        (ONE_HUNDRED_PCT_APY_SSR, "100% APY compound")
    ];
    
    println!("testing _rpow compute units with different max_ssr values:");
    
    for (max_ssr, label) in max_ssr_values {
        // Create a fresh context for each test
        let mut context = program_test_context().await;
        let authority_keypair = Keypair::new();
        let fee_and_destination_owner = Pubkey::new_unique();
        
        // Set up a new curve for this max_ssr value
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
        
        // Create redemption rate curve with this specific max_ssr
        create_redemption_rate_curve(
            &mut context,
            &swap_info,
            &authority,
            &authority_keypair,
            token_a_account,
            token_b_account,
            pool_mint,
            fee_account,
            destination_account,
            max_ssr
        ).await;
        
        println!("\n--- Testing with max_ssr = {} ({}) ---", max_ssr, label);
        
        // Test this curve with different durations
        test_rpow_compute_units_with_growing_duration(
            &mut context,
            &swap_info,
            &authority_keypair
        ).await;
    }
}


async fn test_rpow_compute_units_with_growing_duration(
    context: &mut ProgramTestContext,
    swap_info: &Pubkey,
    authority_keypair: &Keypair
) {
    let permission_account = get_permission_pda(
        swap_info, 
        &authority_keypair.pubkey()
    );

    let initial_curve = fetch_redemption_rate_curve(
        &mut context.banks_client, 
        swap_info
    ).await;

    let durations = vec![
        1,              // 1 second
        10,             // 10 seconds
        60,             // 1 minute
        3600,           // 1 hour
        86400,          // 1 day
        604800,         // 1 week
        2592000,        // 1 month
        15552000,       // 6 months
        31536000,       // 1 year
        315360000,      // 10 years
        3153600000,      // 100 years, overflows!
    ];
    
    println!("testing _rpow compute units with different durations");

    for duration in durations {
        let mut clock = Clock::default();
        clock.unix_timestamp = (initial_curve.rho + duration) as i64;
        context.set_sysvar(&clock);
        
        // For 5% APY
        let ssr = FIVE_PCT_APY_SSR;
        let rho = clock.unix_timestamp as u128;
        let chi = RAY;

        let update_data = vec![
            vec![6], // update discriminator
            ssr.to_le_bytes().to_vec(),
            rho.to_le_bytes().to_vec(),
            chi.to_le_bytes().to_vec(),
        ].concat();

        let accounts = vec![
            AccountMeta::new(*swap_info, false),
            AccountMeta::new_readonly(permission_account, false),
            AccountMeta::new_readonly(authority_keypair.pubkey(), true),
        ];

        let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(1_400_000);


        let ix = Instruction {
            program_id: PROGRAM_ID,
            accounts,
            data: update_data,
        };

        let tx = Transaction::new_signed_with_payer(
            &[compute_budget_ix, ix],
            Some(&context.payer.pubkey()),
            &[&context.payer, authority_keypair],
            context.last_blockhash,
        );

        let cu_used = get_transaction_simulation_cu_used(context, tx).await.unwrap();
        println!("Duration: {} seconds, CU used: {}", duration, cu_used);
    }
}


async fn test_curve_update_valid(
    context: &mut ProgramTestContext,
    swap_info: &Pubkey,
    authority_keypair: &Keypair
) {
    let permission_account = get_permission_pda(
        swap_info, 
        &authority_keypair.pubkey()
    );
    let clock: Clock = context.banks_client.get_sysvar::<Clock>()
        .await
        .unwrap();

    let accounts = vec![
        AccountMeta::new(*swap_info, false),
        AccountMeta::new_readonly(permission_account, false),
        AccountMeta::new_readonly(authority_keypair.pubkey(), true),
    ];

    let update_data = vec![
        // update discriminator
        vec![6],
        (2 * RAY).to_le_bytes().to_vec(),
        (clock.unix_timestamp as u128).to_le_bytes().to_vec(),
        (2 * RAY).to_le_bytes().to_vec(),
    ]
    .concat();

    let ix = Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data: update_data,
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, authority_keypair],
        context.last_blockhash,
    );

    context.banks_client.process_transaction(tx).await.unwrap();

    let updated_curve = fetch_redemption_rate_curve(
        &mut context.banks_client, 
        swap_info
    ).await;

    assert_eq!(updated_curve.ssr, 2 * RAY);
    assert_eq!(updated_curve.chi, 2 * RAY);
}

async fn test_curve_update_invalid_rho(
    context: &mut ProgramTestContext,
    swap_info: &Pubkey,
    authority_keypair: &Keypair
) {
    let permission_account = get_permission_pda(
        swap_info, 
        &authority_keypair.pubkey()
    );
    let clock: Clock = context.banks_client.get_sysvar::<Clock>().await.unwrap();

    let accounts = vec![
        AccountMeta::new(*swap_info, false),
        AccountMeta::new_readonly(permission_account, false),
        AccountMeta::new_readonly(authority_keypair.pubkey(), true),
    ];

    let update_data = vec![
        // update discriminator
        vec![6],
        (2 * RAY).to_le_bytes().to_vec(),
        (clock.unix_timestamp as u128 - 1).to_le_bytes().to_vec(),
        (2 * RAY).to_le_bytes().to_vec(),
    ]
    .concat();

    let ix = Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data: update_data,
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, authority_keypair],
        context.last_blockhash,
    );

    let result = context.banks_client
        .process_transaction(tx)
        .await;

    assert!(result.is_err());
}

async fn test_permission_metadata(
    context: &mut ProgramTestContext,
    permission_account: &Pubkey,
    swap_info: &Pubkey,
    authority_keypair: &Keypair
) {
    let permission = fetch_permission(
        &mut context.banks_client, 
        permission_account
    ).await;
    assert!(permission.can_update_parameters);
    assert!(permission.is_super_admin);
    assert_eq!(permission.swap, *swap_info);
    assert_eq!(permission.authority, authority_keypair.pubkey());
}

async fn test_permission_rejection_unauthorized_update(
    context: &mut ProgramTestContext,
    swap_info: &Pubkey,
    permission_account: &Pubkey
) {
    let fake = Keypair::new();
    let clock: Clock = context.banks_client
        .get_sysvar::<Clock>()
        .await
        .unwrap();

    let accounts = vec![
        AccountMeta::new(*swap_info, false),
        AccountMeta::new_readonly(*permission_account, false),
        AccountMeta::new_readonly(fake.pubkey(), true),
    ];

    let data = vec![
        // update discriminator
        vec![6],
        (2 * RAY).to_le_bytes().to_vec(),
        (clock.unix_timestamp as u128).to_le_bytes().to_vec(),
        (2 * RAY).to_le_bytes().to_vec(),
    ]
    .concat();

    let ix = Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data,
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &fake],
        context.last_blockhash,
    );

    let result = context.banks_client
        .process_transaction(tx)
        .await;

    assert!(result.is_err());
}

async fn test_permission_grant_and_update(
    context: &mut ProgramTestContext,
    swap_info: &Pubkey,
    permission_account: &Pubkey,
    authority_keypair: &Keypair
) {
    let new_auth = Keypair::new();

    // Create new permission (can update, not super)
    let new_permission = get_permission_pda(
        swap_info, 
        &new_auth.pubkey()
    );

    let init_data = vec![
        // init permission discrriminator
        vec![7],
        // new authority
        new_auth.pubkey().to_bytes().to_vec(),
        // is_super_admin
        vec![false as u8],
        // can_update_parameters
        vec![true as u8],
    ]
    .concat();

    let accounts = vec![
        AccountMeta::new_readonly(*swap_info, false),
        AccountMeta::new_readonly(*permission_account, false),
        AccountMeta::new(new_permission, false),
        AccountMeta::new_readonly(authority_keypair.pubkey(), true),
        AccountMeta::new_readonly(context.payer.pubkey(), true),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
    ];

    let tx = Transaction::new_signed_with_payer(
        &[Instruction {
            program_id: PROGRAM_ID,
            accounts,
            data: init_data,
        }],
        Some(&context.payer.pubkey()),
        &[&context.payer, authority_keypair],
        context.last_blockhash,
    );

    context.banks_client
        .process_transaction(tx)
        .await
        .unwrap();

    let fetched = fetch_permission(
        &mut context.banks_client, 
        &new_permission
    ).await;

    assert!(fetched.can_update_parameters);
    assert!(!fetched.is_super_admin);

    // Now upgrade to super admin
    let upgrade_data = vec![
        // update permission discriminator
        vec![8],
        // is_super_admin
         vec![true as u8], 
         // can_update_parameters
         vec![true as u8]
         ].concat();

    let upgrade_accounts = vec![
        AccountMeta::new_readonly(*swap_info, false),
        AccountMeta::new_readonly(*permission_account, false),
        AccountMeta::new(new_permission, false),
        AccountMeta::new_readonly(authority_keypair.pubkey(), true),
    ];

    let tx = Transaction::new_signed_with_payer(
        &[Instruction {
            program_id: PROGRAM_ID,
            accounts: upgrade_accounts,
            data: upgrade_data,
        }],
        Some(&context.payer.pubkey()),
        &[&context.payer, authority_keypair],
        context.last_blockhash,
    );

    context.banks_client
        .process_transaction(tx)
        .await
        .unwrap();

    let upgraded = fetch_permission(
        &mut context.banks_client, 
        &new_permission
    ).await;

    assert!(upgraded.is_super_admin);
}

async fn create_redemption_rate_curve(
    context: &mut ProgramTestContext,
    swap_info: &Pubkey,
    authority: &Pubkey,
    authority_keypair: &Keypair,
    token_a_account: Pubkey,
    token_b_account: Pubkey,
    pool_mint: Pubkey,
    fee_account: Pubkey,
    destination_account: Pubkey,
    max_ssr: u128
) {
    let permission_account = get_permission_pda(
        swap_info, 
        &authority_keypair.pubkey()
    );

    let init_accounts = vec![
        AccountMeta::new(*swap_info, false),
        AccountMeta::new_readonly(*authority, false),
        AccountMeta::new_readonly(token_a_account, false),
        AccountMeta::new_readonly(token_b_account, false),
        AccountMeta::new(pool_mint, false),
        AccountMeta::new_readonly(fee_account, false),
        AccountMeta::new(destination_account, false),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        AccountMeta::new(permission_account, false),
        AccountMeta::new_readonly(authority_keypair.pubkey(), false),
        AccountMeta::new(context.payer.pubkey(), true),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
    ];

    let fees = Fees::default();
    let mut fees_buf = vec![0; 64];
    fees.pack_into_slice(&mut fees_buf);

    let clock: Clock = context.banks_client
        .get_sysvar::<Clock>()
        .await
        .unwrap();

    let curve = RedemptionRateCurve {
        ray: RAY,
        max_ssr,
        ssr: RAY,
        rho: clock.unix_timestamp as u128,
        chi: RAY,
    };

    let mut curve_buf = vec![0; 80];
    Pack::pack_into_slice(&curve, &mut curve_buf);

    let data = vec![
        // init curve discriminator
        vec![0],        
        fees_buf,
        // RedemptionRateCurve
        vec![3],
        curve_buf,
    ]
    .concat();

    let ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: init_accounts,
        data,
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );

    context.banks_client
        .process_transaction(tx)
        .await
        .unwrap();

    let result = fetch_redemption_rate_curve(
        &mut context.banks_client, 
        swap_info
    ).await;

    assert_eq!(result, curve);
}